use super::{EXECUTABLE_ADDRESS, HHDM, MEMORY_MAP, check_protocol};
use kryo_core::mm::frame::{
    FRAME_SIZE, FramePool, MemoryRegion, NormalizeError, PhysRange, RegionKind, normalize_memory,
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

const BITMAP_WORDS: usize = 16_384;
const SUMMARY_WORDS: usize = BITMAP_WORDS.div_ceil(64);
pub(super) type BootFramePool = FramePool<OUTPUT_CAPACITY, BITMAP_WORDS, SUMMARY_WORDS>;
static FRAMES: BootFramePool = FramePool::new();

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

    let reservation_count = 0;

    let plan = match normalize_memory(
        &mut regions[..count],
        &mut reservations[..reservation_count],
        available,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            let _ = kryo_hal::print(format_args!("MEM normalize error: {:?}\n", error));
            panic!("Cannot normalize boot memory");
        }
    };

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

    // Only unreserved Usable RAM enters the sole pool; static metadata is in the kernel image.
    match unsafe { FRAMES.initialize(plan) } {
        Ok(()) => {}
        Err(error) => {
            let _ = kryo_hal::print(format_args!("FRAME init error: {:?}\n", error));
            panic!("Cannot initialize physical frames");
        }
    }
    drop(storage);
    let stats = FRAMES.stats();
    let _ = kryo_hal::print(format_args!(
        "FRAME: ready total={} free={}\n",
        stats.total, stats.free,
    ));
    #[cfg(feature = "boot-tests")]
    {
        let hhdm = HHDM.response().expect("Missing Limine HHDM response");
        // Limine maps Usable RAM and its page tables through the retained HHDM.
        let direct_map =
            unsafe { kryo_hal::DirectMap::new(hhdm.offset) }.expect("Invalid Limine direct map");
        super::checks::run(&FRAMES, direct_map);
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
        super::user_checks::run(&FRAMES, direct_map, physical_end);
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
