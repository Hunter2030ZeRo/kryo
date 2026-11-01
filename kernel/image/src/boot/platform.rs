use crate::runtime::KernelRegion;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use kryo_hal::DirectMap;

const PAGE_SIZE: u64 = 4096;
const EMPTY: u8 = 0;
const PREPARING: u8 = 1;
const READY: u8 = 2;
const TAKEN: u8 = 3;

#[repr(C, align(4096))]
struct TablePages(UnsafeCell<[[u8; PAGE_SIZE as usize]; 3]>);

// No Rust reference accesses these bytes. The BSP transfers their exclusive
// page-table use to KernelRegion once; the kernel image retains them forever.
unsafe impl Sync for TablePages {}

struct RegionSlot(UnsafeCell<Option<KernelRegion>>);

// Only the BSP with IRQs disabled accesses this slot. APs read the published
// mapping address, never the KernelRegion owner or its mutable page-table state.
unsafe impl Sync for RegionSlot {}

static TABLES: TablePages = TablePages(UnsafeCell::new([[0; PAGE_SIZE as usize]; 3]));
static REGION: RegionSlot = RegionSlot(UnsafeCell::new(None));
static STATE: AtomicU8 = AtomicU8::new(EMPTY);
static APIC_MAPPING: AtomicU64 = AtomicU64::new(0);
static ROOT_FRAME: AtomicU64 = AtomicU64::new(0);

/// Establishes the retained APIC mapping before APs leave their trampolines.
pub(crate) fn prepare() {
    check_bsp();
    STATE
        .compare_exchange(EMPTY, PREPARING, Ordering::AcqRel, Ordering::Acquire)
        .expect("Platform region already prepared");
    let hhdm = super::HHDM.response().expect("Missing Limine HHDM");
    // Limine retains the HHDM aliases for its boot root and kernel-image RAM.
    let direct = unsafe { DirectMap::new(hhdm.offset) }.expect("Invalid platform direct map");
    let tables = table_frames();
    // Static image pages are exclusive table storage, excluded from the frame
    // pool. Alias policy is established later, before any user root is cloned.
    let mut region = unsafe { KernelRegion::new(direct, crate::runtime::REGION_BASE, tables) }
        .expect("Cannot attach platform region");
    let physical = kryo_hal::timer_physical_address().expect("Unsupported platform APIC");
    // The platform owns this mapping through AP shutdown. No ordinary RAM
    // references cover APIC registers; every controller uses the same UC alias.
    let mapped = unsafe { region.map_mmio(0, physical) }.expect("Cannot map platform APIC");
    unsafe { kryo_hal::init_timer(mapped) }.expect("Cannot initialize platform APIC");
    unsafe { *REGION.0.get() = Some(region) };
    ROOT_FRAME.store(
        x86_64::registers::control::Cr3::read()
            .0
            .start_address()
            .as_u64(),
        Ordering::Release,
    );
    APIC_MAPPING.store(mapped, Ordering::Release);
    STATE.store(READY, Ordering::Release);
}

pub(crate) fn apic_mapping() -> u64 {
    let mapped = APIC_MAPPING.load(Ordering::Acquire);
    assert_ne!(mapped, 0, "Platform APIC is not prepared");
    mapped
}

pub(crate) fn root_frame() -> u64 {
    let frame = ROOT_FRAME.load(Ordering::Acquire);
    assert_ne!(frame, 0, "Platform root is not prepared");
    frame
}

/// Transfers the branch owner once. Its image-backed table pages never enter
/// the frame pool, including after the caller retires every root and detaches.
pub(crate) fn take_region() -> KernelRegion {
    check_bsp();
    STATE
        .compare_exchange(READY, TAKEN, Ordering::AcqRel, Ordering::Acquire)
        .expect("Platform region is unavailable or already taken");
    unsafe { (*REGION.0.get()).take() }.expect("Platform region ownership invariant")
}

fn table_frames() -> [u64; 3] {
    let image = super::EXECUTABLE_ADDRESS
        .response()
        .expect("Missing Limine executable address");
    let offset = (TABLES.0.get() as u64)
        .checked_sub(image.virtual_base)
        .expect("Platform tables precede executable base");
    let physical = image
        .physical_base
        .checked_add(offset)
        .expect("Platform table address overflow");
    assert!(
        physical.is_multiple_of(PAGE_SIZE),
        "Unaligned platform tables"
    );
    let entries = super::MEMORY_MAP
        .response()
        .expect("Missing Limine memory map");
    core::array::from_fn(|index| {
        let frame = physical
            .checked_add(index as u64 * PAGE_SIZE)
            .expect("Platform table address overflow");
        let end = frame
            .checked_add(PAGE_SIZE)
            .expect("Platform table end overflow");
        assert!(
            entries.entries().iter().any(|entry| {
                entry.type_ == limine::memmap::MEMMAP_EXECUTABLE_AND_MODULES
                    && entry.base <= frame
                    && entry
                        .base
                        .checked_add(entry.length)
                        .is_some_and(|last| end <= last)
            }),
            "Platform table page is outside retained executable RAM"
        );
        frame
    })
}

fn check_bsp() {
    assert!(!x86_64::instructions::interrupts::are_enabled());
    assert_eq!(kryo_hal::current_cpu().map(|cpu| cpu.index()), Some(0));
}
