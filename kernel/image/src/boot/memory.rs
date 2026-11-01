use super::{EXECUTABLE_ADDRESS, HHDM, MEMORY_MAP, check_protocol};
use core::sync::atomic::{AtomicBool, Ordering};
use kryo_core::mm::frame::{
    FRAME_SIZE, FrameMetadata, FrameMetadataLayout, FramePool, MemoryRegion, NormalizeError,
    PhysRange, RegionKind, metadata_layout, normalize_memory,
};
use limine::memmap as lm;

const REGION_CAPACITY: usize = 256;
const RESERVATION_CAPACITY: usize = 32;
const OUTPUT_CAPACITY: usize = REGION_CAPACITY + RESERVATION_CAPACITY;

struct BootMemoryStorage {
    regions: [MemoryRegion; REGION_CAPACITY],
    reservations: [PhysRange; RESERVATION_CAPACITY],
    available: [PhysRange; OUTPUT_CAPACITY],
}

static BOOT_MEMORY: spin::Mutex<BootMemoryStorage> = spin::Mutex::new(BootMemoryStorage {
    regions: [MemoryRegion::EMPTY; REGION_CAPACITY],
    reservations: [PhysRange::EMPTY; RESERVATION_CAPACITY],
    available: [PhysRange::EMPTY; OUTPUT_CAPACITY],
});

pub(crate) type BootFramePool = FramePool<OUTPUT_CAPACITY, 0, 0>;
static FRAMES: BootFramePool = FramePool::new();
static MEMORY_STARTED: AtomicBool = AtomicBool::new(false);

fn decode_memory_region(entry: &lm::Entry) -> Result<MemoryRegion, NormalizeError> {
    let range = PhysRange::from_base_len(entry.base, entry.length)
        .ok_or(NormalizeError::AddressOverflow)?;

    let kind = match entry.type_ {
        lm::MEMMAP_USABLE => RegionKind::Usable,
        lm::MEMMAP_RESERVED => RegionKind::Reserved,
        lm::MEMMAP_ACPI_RECLAIMABLE => RegionKind::AcpiReclaimable,
        lm::MEMMAP_ACPI_NVS => RegionKind::AcpiNvs,
        lm::MEMMAP_BAD_MEMORY => RegionKind::BadMemory,
        lm::MEMMAP_BOOTLOADER_RECLAIMABLE => RegionKind::BootloaderReclaimable,
        lm::MEMMAP_EXECUTABLE_AND_MODULES => RegionKind::ExecutableAndModules,
        lm::MEMMAP_FRAMEBUFFER => RegionKind::Framebuffer,
        lm::MEMMAP_MAPPED_RESERVED => RegionKind::MappedReserved,
        other => RegionKind::Unknown(other),
    };

    Ok(MemoryRegion { range, kind })
}

pub(crate) fn init_physical_memory() {
    check_protocol();
    // Claim bootstrap ownership before carving or zeroing metadata. Retrying
    // would otherwise overwrite storage already borrowed by the active pool.
    MEMORY_STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .expect("Physical memory bootstrap already started");
    log_boot_memory();

    let Some(map) = MEMORY_MAP.response() else {
        panic!("Missing Limine memory map");
    };
    let count = map.entries().len();

    if count > REGION_CAPACITY {
        panic!("Boot memory region capacity exceeded");
    }

    let mut storage = BOOT_MEMORY.lock();
    let BootMemoryStorage {
        regions,
        reservations,
        available,
    } = &mut *storage;

    for (slot, entry) in regions[..count].iter_mut().zip(map.entries()) {
        *slot = match decode_memory_region(entry) {
            Ok(region) => region,
            Err(_) => panic!("Invalid boot memory range"),
        };
    }

    let (metadata_range, layout, usable_frames) = {
        let plan = normalize_memory(&mut regions[..count], &mut [], available)
            .expect("Cannot normalize boot memory");
        check_physical_width(plan.available());
        let layout = metadata_layout(plan.frame_count()).expect("Invalid frame metadata size");
        let bytes = u64::try_from(layout.bytes)
            .expect("Frame metadata exceeds address width")
            .checked_add(FRAME_SIZE - 1)
            .expect("Frame metadata size overflow")
            & !(FRAME_SIZE - 1);
        let range = plan
            .available()
            .iter()
            .find(|range| range.len() >= bytes)
            .and_then(|range| PhysRange::from_base_len(range.start(), bytes))
            .expect("No contiguous Usable range for frame metadata");
        (range, layout, plan.frame_count())
    };
    reservations[0] = metadata_range;
    let plan = normalize_memory(&mut regions[..count], &mut reservations[..1], available)
        .expect("Cannot reserve frame metadata");
    assert_eq!(
        usable_frames - plan.frame_count(),
        metadata_range.len() / FRAME_SIZE
    );
    assert!(
        plan.available().iter().all(|range| {
            range.end() <= metadata_range.start() || range.start() >= metadata_range.end()
        }),
        "Frame metadata remains allocatable"
    );
    let hhdm = HHDM.response().expect("Missing Limine HHDM response");
    // Limine retains the original Usable RAM aliases, including this reserved span.
    let direct = unsafe { kryo_hal::DirectMap::new(hhdm.offset) }.expect("Invalid boot direct map");
    #[cfg(feature = "boot-tests")]
    let high_ranges = [1 << 32, 1 << 37].map(|minimum| {
        plan.available().iter().find_map(|range| {
            let first = range.start().max(minimum);
            (first < range.end()).then(|| PhysRange::from_base_len(first, FRAME_SIZE).unwrap())
        })
    });
    let _ = kryo_hal::print(format_args!(
        "FRAME metadata: start={:#x} end={:#x} bytes={} bitmap_words={} summary_words={} usable_frames={} managed_frames={}\n",
        metadata_range.start(),
        metadata_range.end(),
        metadata_range.len(),
        layout.bitmap_words,
        layout.summary_words,
        usable_frames,
        plan.frame_count(),
    ));
    let _ = kryo_hal::print(format_args!(
        "MEM normalized: ranges={} frames={}\n",
        plan.available().len(),
        plan.frame_count(),
    ));

    for (index, range) in plan.available().iter().enumerate() {
        let _ = kryo_hal::print(format_args!(
            "AVAILABLE[{}]: [{:#018x}, {:#018x}) frames={}\n",
            index,
            range.start(),
            range.end(),
            range.len() / FRAME_SIZE,
        ));
    }

    // The boot reservation owns these bytes permanently; the pool never allocates them.
    let metadata = unsafe { metadata_storage(direct, metadata_range, layout) };
    if let Err(failure) = unsafe { FRAMES.initialize_external(plan, metadata) } {
        let _ = kryo_hal::print(format_args!("FRAME init error: {:?}\n", failure.error));
        panic!("Cannot initialize physical frames");
    }
    drop(storage);
    #[cfg(feature = "boot-tests")]
    {
        let before = FRAMES.stats();
        assert!(matches!(
            FRAMES.allocate_in(metadata_range),
            Err(kryo_core::mm::frame::FrameAllocError::OutOfMemory)
        ));
        assert_eq!(FRAMES.stats(), before);
        kryo_hal::emergency_write_str("FRAME: metadata reservation excluded\n");
    }
    unsafe { crate::heap::init(&FRAMES, direct) };
    #[cfg(feature = "boot-tests")]
    crate::heap::check();
    let stats = FRAMES.stats();
    let _ = kryo_hal::print(format_args!(
        "FRAME: ready total={} free={}\n",
        stats.total, stats.free,
    ));
    #[cfg(feature = "boot-tests")]
    {
        super::checks::check_high_memory(&FRAMES, direct, high_ranges);
        super::checks::run(&FRAMES, direct);
        let physical_end = map
            .entries()
            .iter()
            .filter(|entry| entry.type_ == lm::MEMMAP_USABLE)
            .map(|entry| {
                entry
                    .base
                    .checked_add(entry.length)
                    .expect("Overflow in usable RAM")
            })
            .max()
            .expect("No usable RAM");
        super::user_checks::run(&FRAMES, direct, physical_end);
    }
}

fn check_physical_width(ranges: &[PhysRange]) {
    let maximum = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    let bits = if maximum >= 0x8000_0008 {
        core::arch::x86_64::__cpuid(0x8000_0008).eax & 0xff
    } else {
        36
    };
    assert!(
        (1..=52).contains(&bits),
        "Unsupported physical address width"
    );
    assert!(
        ranges.iter().all(|range| (range.end() - 1) >> bits == 0),
        "Usable RAM exceeds CPU physical address width"
    );
}

/// # Safety
/// The normalized boot plan granted exclusive ownership of this retained HHDM
/// span and excluded it from the sole frame pool. No other aliases or allocator
/// may access its bytes independently for the remaining kernel lifetime.
unsafe fn metadata_storage(
    direct: kryo_hal::DirectMap,
    range: PhysRange,
    layout: FrameMetadataLayout,
) -> FrameMetadata {
    let first = direct
        .frame_address(range.start())
        .expect("Invalid metadata mapping");
    let last = direct
        .frame_address(range.end() - FRAME_SIZE)
        .expect("Invalid metadata end");
    assert_eq!(last.checked_add(FRAME_SIZE).unwrap() - first, range.len());
    let bytes = usize::try_from(range.len()).expect("Metadata span exceeds address width");
    assert!(layout.bytes <= bytes);
    let pointer = first as *mut u64;
    // Initialize before creating typed references; both slices are disjoint and
    // entirely within the reserved pages, including their alignment padding.
    unsafe {
        pointer.cast::<u8>().write_bytes(0, bytes);
        let bitmap = core::slice::from_raw_parts_mut(pointer, layout.bitmap_words);
        let summary =
            core::slice::from_raw_parts_mut(pointer.add(layout.bitmap_words), layout.summary_words);
        FrameMetadata::new(bitmap, summary)
    }
}

fn log_boot_memory() {
    check_protocol();

    let Some(map) = MEMORY_MAP.response() else {
        panic!("Missing Limine memory map");
    };

    let Some(hhdm) = HHDM.response() else {
        panic!("Missing Limine HHDM response");
    };

    let Some(image) = EXECUTABLE_ADDRESS.response() else {
        panic!("Missing Limine executable address");
    };

    let _ = kryo_hal::print(format_args!(
        "BOOT: hhdm={:#018x} kernel_phys={:#018x} kernel_virt={:#018x}\n",
        hhdm.offset, image.physical_base, image.virtual_base,
    ));

    for (index, entry) in map.entries().iter().enumerate() {
        let Some(end) = entry.base.checked_add(entry.length) else {
            panic!("Overflow in Limine memmap");
        };

        let _ = kryo_hal::print(format_args!(
            "MEM[{}]: kind={} [{:#018x}, {:#018x}) bytes={}\n",
            index, entry.type_, entry.base, end, entry.length,
        ));
    }
}

#[cfg(not(feature = "boot-tests"))]
pub(crate) fn frames() -> &'static BootFramePool {
    &FRAMES
}
