use std::cell::{Cell, RefCell};
use std::rc::Rc;

use kryo_core::device::dma::{DmaBackend, DmaBuffer, DmaError, DmaLimits, DmaState};
use kryo_core::mm::frame::{
    FRAME_SIZE, FrameAllocError, FramePool, MemoryRegion, PhysRange, RegionKind, normalize_memory,
};

type Pool = FramePool<3, 2, 1>;

fn span(start: u64, pages: u64) -> PhysRange {
    PhysRange::from_base_len(start, pages * FRAME_SIZE).unwrap()
}

fn pool(regions: &[PhysRange]) -> Pool {
    let pool = Pool::new();
    let mut regions: Vec<_> = regions
        .iter()
        .map(|&range| MemoryRegion {
            range,
            kind: RegionKind::Usable,
        })
        .collect();
    let mut available = [PhysRange::EMPTY; 3];
    let plan = normalize_memory(&mut regions, &mut [], &mut available).unwrap();
    // These tests model ownership without dereferencing physical RAM addresses.
    unsafe { pool.initialize(plan) }.unwrap();
    pool
}

fn limits() -> DmaLimits {
    DmaLimits {
        max_pages: 4,
        physical: span(0x1000, 16),
        address_limit: u32::MAX.into(),
    }
}

#[derive(Debug, Default)]
struct Trace {
    events: RefCell<Vec<&'static str>>,
    quiesce_failures: Cell<u8>,
    panic_quiesce: Cell<bool>,
    unmap_failures: Cell<u8>,
    backend_drops: Cell<u8>,
    mapping_drops: Cell<u8>,
    initialized: Cell<bool>,
}

#[derive(Debug)]
struct Backend {
    trace: Rc<Trace>,
    address: Option<u64>,
    fail_initialize: bool,
    fail_map: bool,
}

struct Mapping {
    trace: Rc<Trace>,
    address: u64,
}

impl Backend {
    fn new(trace: Rc<Trace>) -> Self {
        Self {
            trace,
            address: None,
            fail_initialize: false,
            fail_map: false,
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.trace
            .backend_drops
            .set(self.trace.backend_drops.get() + 1);
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        self.trace
            .mapping_drops
            .set(self.trace.mapping_drops.get() + 1);
    }
}

// This backend models mapping/retirement only. It never touches hardware, and
// every modeled device transaction ends when quiesce reports success.
unsafe impl DmaBackend for Backend {
    type Mapping = Mapping;
    type Error = &'static str;

    unsafe fn initialize(&mut self, _: u64, _: u64) -> Result<(), Self::Error> {
        self.trace.events.borrow_mut().push("initialize");
        if self.fail_initialize {
            return Err("initialize");
        }
        self.trace.initialized.set(true);
        Ok(())
    }

    unsafe fn map(&mut self, physical: u64, _: u64) -> Result<Mapping, Self::Error> {
        self.trace.events.borrow_mut().push("map");
        assert!(self.trace.initialized.get());
        if self.fail_map {
            return Err("map");
        }
        Ok(Mapping {
            trace: self.trace.clone(),
            address: self.address.unwrap_or(physical + 0x10_0000),
        })
    }

    fn address(&self, mapping: &Mapping) -> u64 {
        mapping.address
    }

    fn quiesce(&mut self, _: &mut Mapping) -> Result<(), Self::Error> {
        self.trace.events.borrow_mut().push("quiesce");
        assert!(!self.trace.panic_quiesce.get(), "injected quiesce panic");
        let remaining = self.trace.quiesce_failures.get();
        if remaining > 0 {
            self.trace.quiesce_failures.set(remaining - 1);
            return Err("busy");
        }
        Ok(())
    }

    fn unmap(&mut self, _: &mut Mapping) -> Result<(), Self::Error> {
        self.trace.events.borrow_mut().push("unmap");
        let remaining = self.trace.unmap_failures.get();
        if remaining > 0 {
            self.trace.unmap_failures.set(remaining - 1);
            return Err("iotlb");
        }
        Ok(())
    }
}

#[test]
fn allocation_retains_frames_until_ordered_retirement() {
    let pool = pool(&[span(0x1000, 16)]);
    let trace = Rc::new(Trace::default());
    let mut buffer = DmaBuffer::new(&pool, 3, limits(), Backend::new(trace.clone())).unwrap();
    assert_eq!(pool.stats().free, 13);
    assert_eq!(buffer.physical_address(), 0x1000);
    assert_eq!(buffer.range().address, 0x10_1000);
    assert_eq!(buffer.range().bytes, 3 * FRAME_SIZE);
    assert_eq!(buffer.state(), DmaState::Mapped);
    let range = buffer.publish().unwrap();
    assert_eq!(range, buffer.range());
    assert_eq!(buffer.state(), DmaState::InFlight);
    assert_eq!(buffer.publish(), Err(DmaError::InvalidState));
    drop(buffer.retire().unwrap());
    assert_eq!(pool.stats().free, 16);
    assert_eq!(
        &*trace.events.borrow(),
        &["initialize", "map", "quiesce", "unmap"]
    );
    assert_eq!(trace.backend_drops.get(), 1);
    assert_eq!(trace.mapping_drops.get(), 1);
}

#[test]
fn failed_quiescence_keeps_all_owners_and_retries_before_unmap() {
    let pool = pool(&[span(0x1000, 16)]);
    let trace = Rc::new(Trace::default());
    trace.quiesce_failures.set(1);
    let mut buffer = DmaBuffer::new(&pool, 4, limits(), Backend::new(trace.clone())).unwrap();
    buffer.publish().unwrap();
    let failure = buffer.retire().unwrap_err();
    assert_eq!(failure.error, DmaError::Quiesce("busy"));
    let mut buffer = failure.retained.unwrap();
    assert_eq!(buffer.state(), DmaState::Quiescing);
    assert_eq!(buffer.publish(), Err(DmaError::InvalidState));
    assert_eq!(pool.stats().free, 12);
    assert_eq!(trace.mapping_drops.get(), 0);
    assert_eq!(trace.backend_drops.get(), 0);
    assert!(!trace.events.borrow().contains(&"unmap"));
    drop(buffer.retire().unwrap());
    assert_eq!(pool.stats().free, 16);
    assert_eq!(
        &*trace.events.borrow(),
        &["initialize", "map", "quiesce", "quiesce", "unmap"]
    );
}

#[test]
fn failed_unmap_keeps_backing_and_does_not_repeat_quiescence() {
    let pool = pool(&[span(0x1000, 16)]);
    let trace = Rc::new(Trace::default());
    trace.unmap_failures.set(1);
    let buffer = DmaBuffer::new(&pool, 2, limits(), Backend::new(trace.clone())).unwrap();
    let failure = buffer.retire().unwrap_err();
    assert_eq!(failure.error, DmaError::Unmap("iotlb"));
    let buffer = failure.retained.unwrap();
    assert_eq!(buffer.state(), DmaState::Quiescent);
    assert_eq!(pool.stats().free, 14);
    assert_eq!(trace.mapping_drops.get(), 0);
    drop(buffer.retire().unwrap());
    assert_eq!(pool.stats().free, 16);
    assert_eq!(
        &*trace.events.borrow(),
        &["initialize", "map", "quiesce", "unmap", "unmap"]
    );
}

#[test]
fn abandoning_an_inflight_owner_never_returns_frames_or_drops_translation() {
    let pool = pool(&[span(0x1000, 16)]);
    let trace = Rc::new(Trace::default());
    let mut buffer = DmaBuffer::new(&pool, 4, limits(), Backend::new(trace.clone())).unwrap();
    buffer.publish().unwrap();
    drop(buffer);
    assert_eq!(pool.stats().free, 12);
    assert_eq!(trace.backend_drops.get(), 0);
    assert_eq!(trace.mapping_drops.get(), 0);
    assert_eq!(&*trace.events.borrow(), &["initialize", "map"]);
}

#[test]
fn unpublished_owner_is_also_explicitly_retired() {
    let pool = pool(&[span(0x1000, 16)]);
    let trace = Rc::new(Trace::default());
    let buffer = DmaBuffer::new(&pool, 1, limits(), Backend::new(trace.clone())).unwrap();
    drop(buffer);
    assert_eq!(pool.stats().free, 15);
    assert_eq!(trace.backend_drops.get(), 0);
    assert_eq!(trace.mapping_drops.get(), 0);
}

#[test]
fn invalid_sizes_fail_before_allocating_or_touching_backend() {
    let pool = pool(&[span(0x1000, 16)]);
    for pages in [0, 5, usize::MAX] {
        let trace = Rc::new(Trace::default());
        let failure = DmaBuffer::new(&pool, pages, limits(), Backend::new(trace.clone()))
            .err()
            .unwrap();
        assert_eq!(failure.error, DmaError::InvalidSize);
        assert!(failure.retained.is_none());
        assert!(trace.events.borrow().is_empty());
        assert_eq!(pool.stats().free, 16);
    }
}

#[test]
fn invalid_physical_limits_grant_no_authority() {
    let pool = pool(&[span(0x1000, 16)]);
    for physical in [
        PhysRange::EMPTY,
        PhysRange::from_base_len(0x1001, 4096).unwrap(),
    ] {
        let failure = DmaBuffer::new(
            &pool,
            1,
            DmaLimits {
                physical,
                ..limits()
            },
            Backend::new(Rc::default()),
        )
        .err()
        .unwrap();
        assert_eq!(failure.error, DmaError::InvalidLimits);
        assert!(failure.retained.is_none());
    }
    let failure = DmaBuffer::new(
        &pool,
        1,
        DmaLimits {
            physical: span(0x10_0000, 1),
            ..limits()
        },
        Backend::new(Rc::default()),
    )
    .err()
    .unwrap();
    assert_eq!(
        failure.error,
        DmaError::Allocation(FrameAllocError::OutOfMemory)
    );
    assert_eq!(pool.stats().free, 16);
}

#[test]
fn initialization_and_mapping_errors_return_unpublished_backing() {
    let pool = pool(&[span(0x1000, 16)]);
    for initialize in [true, false] {
        let trace = Rc::new(Trace::default());
        let mut backend = Backend::new(trace.clone());
        backend.fail_initialize = initialize;
        backend.fail_map = !initialize;
        let failure = DmaBuffer::new(&pool, 3, limits(), backend).err().unwrap();
        assert_eq!(
            failure.error,
            if initialize {
                DmaError::Initialize("initialize")
            } else {
                DmaError::Map("map")
            }
        );
        assert!(failure.retained.is_none());
        assert_eq!(pool.stats().free, 16);
        assert_eq!(trace.backend_drops.get(), 1);
        assert_eq!(trace.mapping_drops.get(), 0);
        assert!(!trace.events.borrow().contains(&"quiesce"));
    }
}

#[test]
fn device_mask_and_overflow_reject_entire_range_and_retire_mapping() {
    let pool = pool(&[span(0x1000, 16)]);
    for address in [u64::from(u32::MAX) - FRAME_SIZE + 2, u64::MAX - 1] {
        let trace = Rc::new(Trace::default());
        let mut backend = Backend::new(trace.clone());
        backend.address = Some(address);
        let failure = DmaBuffer::new(&pool, 1, limits(), backend).err().unwrap();
        assert_eq!(failure.error, DmaError::AddressLimit);
        assert!(failure.retained.is_none());
        assert_eq!(pool.stats().free, 16);
        assert_eq!(
            &*trace.events.borrow(),
            &["initialize", "map", "quiesce", "unmap"]
        );
    }
}

#[test]
fn constructor_cleanup_failure_returns_reachable_owner() {
    let pool = pool(&[span(0x1000, 16)]);
    let trace = Rc::new(Trace::default());
    trace.unmap_failures.set(1);
    let mut backend = Backend::new(trace.clone());
    backend.address = Some(u64::MAX);
    let failure = DmaBuffer::new(&pool, 2, limits(), backend).err().unwrap();
    assert_eq!(failure.error, DmaError::AddressLimit);
    let buffer = failure.retained.unwrap();
    assert_eq!(buffer.state(), DmaState::Quiescent);
    assert_eq!(pool.stats().free, 14);
    assert_eq!(trace.backend_drops.get(), 0);
    assert_eq!(trace.mapping_drops.get(), 0);
    drop(buffer.retire().unwrap());
    assert_eq!(pool.stats().free, 16);
}

#[test]
fn inclusive_device_limit_accepts_final_byte() {
    let pool = pool(&[span(0x1000, 16)]);
    let mut backend = Backend::new(Rc::default());
    backend.address = Some(u64::from(u32::MAX) - FRAME_SIZE + 1);
    let buffer = DmaBuffer::new(&pool, 1, limits(), backend).unwrap();
    drop(buffer.retire().unwrap());
    assert_eq!(pool.stats().free, 16);
}

#[test]
fn constrained_runs_preserve_holes_occupied_frames_and_exact_bounds() {
    let pool = pool(&[span(0x1000, 4), span(0x10_000, 8)]);
    let occupied = pool.allocate_in(span(0x11_000, 1)).unwrap();
    let before = pool.stats();
    assert!(matches!(
        pool.allocate_run_in(3, span(0x10_000, 3)),
        Err(FrameAllocError::OutOfMemory)
    ));
    assert_eq!(pool.stats(), before);
    let run = pool.allocate_run_in(3, span(0x11_000, 4)).unwrap();
    assert_eq!(run.address(), 0x12_000);
    assert_eq!(run.page_count(), 3);
    assert!(matches!(
        pool.allocate_run_in(5, span(0x1000, 20)),
        Err(FrameAllocError::OutOfMemory)
    ));
    unsafe { pool.release_run(run) }.unwrap();
    unsafe { pool.release(occupied) }.unwrap();
    assert_eq!(pool.stats().free, 12);
}

#[test]
fn constrained_runs_can_allocate_above_four_gib_without_truncation() {
    let base = (1u64 << 32) + 0x1000;
    let pool = pool(&[span(0x1000, 4), span(base, 8)]);
    let run = pool.allocate_run_in(3, span(base + FRAME_SIZE, 3)).unwrap();
    assert_eq!(run.address(), base + FRAME_SIZE);
    unsafe { pool.release_run(run) }.unwrap();
    assert_eq!(pool.stats().free, 12);
}

#[test]
fn constrained_run_invalid_arguments_do_not_change_pool() {
    let pool = pool(&[span(0x1000, 16)]);
    let before = pool.stats();
    for physical in [
        PhysRange::EMPTY,
        PhysRange::from_base_len(0x1000, 4097).unwrap(),
    ] {
        assert!(matches!(
            pool.allocate_run_in(1, physical),
            Err(FrameAllocError::InvalidRange)
        ));
    }
    assert!(matches!(
        pool.allocate_run_in(0, span(0x1000, 1)),
        Err(FrameAllocError::InvalidCount)
    ));
    assert!(matches!(
        pool.allocate_run_in(usize::MAX, span(0x1000, 1)),
        Err(FrameAllocError::InvalidCount)
    ));
    assert_eq!(pool.stats(), before);
}

#[test]
fn panic_during_retirement_retains_backing_and_backend_owners() {
    let pool = pool(&[span(0x1000, 16)]);
    let trace = Rc::new(Trace::default());
    trace.panic_quiesce.set(true);
    let buffer = DmaBuffer::new(&pool, 3, limits(), Backend::new(trace.clone())).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drop(buffer.retire());
    }));
    assert!(result.is_err());
    assert_eq!(pool.stats().free, 13);
    assert_eq!(trace.backend_drops.get(), 0);
    assert_eq!(trace.mapping_drops.get(), 0);
}

#[test]
fn temporary_initializer_borrow_ends_before_device_retention() {
    let pool = pool(&[span(0x1000, 16)]);
    let trace = Rc::new(Trace::default());
    let mut initialized_bytes = 0;
    let buffer = unsafe {
        DmaBuffer::new_with_initializer(
            &pool,
            2,
            limits(),
            Backend::new(trace.clone()),
            |_, bytes| {
                initialized_bytes = bytes;
                trace.initialized.set(true);
                Ok(())
            },
        )
    }
    .unwrap();
    assert_eq!(initialized_bytes, 2 * FRAME_SIZE);
    assert_eq!(&*trace.events.borrow(), &["map"]);
    drop(buffer.retire().unwrap());
    assert_eq!(pool.stats().free, 16);
}

#[test]
fn temporary_initializer_error_returns_unpublished_run_without_mapping() {
    let pool = pool(&[span(0x1000, 16)]);
    let trace = Rc::new(Trace::default());
    let failure = unsafe {
        DmaBuffer::new_with_initializer(&pool, 2, limits(), Backend::new(trace.clone()), |_, _| {
            Err("VM")
        })
    }
    .err()
    .unwrap();
    assert_eq!(failure.error, DmaError::Initialize("VM"));
    assert!(failure.retained.is_none());
    assert!(trace.events.borrow().is_empty());
    assert_eq!(pool.stats().free, 16);
}
