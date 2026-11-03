use kryo_core::mm::frame::{
    FrameAllocError, FrameFreeError, FramePool, MemoryRegion, OwnedFrame, PhysRange, RegionKind,
    normalize_memory,
};
use kryo_core::mm::shared::{SharedBackend, SharedMemory};
use kryo_core::mm::vm::FrameAllocator;
use kryo_core::resource::{Budget, Limits, Resource};

struct RetainedAllocator {
    pool: FramePool<1, 1, 1>,
}

// Host ownership model: every allocation is genuine, but retirement is rejected.
unsafe impl FrameAllocator for RetainedAllocator {
    fn allocate(&self) -> Result<OwnedFrame<'_>, FrameAllocError> {
        self.pool.allocate()
    }

    unsafe fn release<'pool>(
        &self,
        frame: OwnedFrame<'pool>,
    ) -> Result<(), (FrameFreeError, OwnedFrame<'pool>)> {
        Err((FrameFreeError::WrongPool, frame))
    }
}

struct Initializer;

// This model exposes no physical access or mappings, only frame-token ownership.
unsafe impl SharedBackend for Initializer {
    type Error = ();

    unsafe fn initialize_shared(&mut self, _: u64) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[test]
fn failed_backing_retirement_cannot_refund_a_shared_page_charge_during_unwind() {
    let allocator = RetainedAllocator {
        pool: FramePool::new(),
    };
    let mut regions = [MemoryRegion {
        range: PhysRange::from_base_len(0x1000, 0x3000).unwrap(),
        kind: RegionKind::Usable,
    }];
    let mut reservations = [];
    let mut output = [PhysRange::EMPTY; 1];
    let plan = normalize_memory(&mut regions, &mut reservations, &mut output).unwrap();
    unsafe { allocator.pool.initialize(plan) }.unwrap();
    let budget = Budget::new(Limits {
        shared_pages: 1,
        private_pages: 0,
        reserved_pages: 0,
        handles: 0,
        waits: 0,
        threads: 0,
        mappings: 0,
        tables: 0,
    });
    let charge = budget.try_charge(Resource::SharedPages, 1).unwrap();
    let memory =
        SharedMemory::<_, 1>::new_charged(&allocator, 1, &mut Initializer, charge).unwrap();
    let retained = allocator.pool.stats();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(memory)));
    assert!(failure.is_err());
    assert_eq!(allocator.pool.stats(), retained);
    assert_eq!(budget.count(Resource::SharedPages), 1);
    assert!(budget.try_charge(Resource::SharedPages, 1).is_err());
}
