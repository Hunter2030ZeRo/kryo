use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use kryo_core::capability::{CapabilityError, HandleTable, Rights};
use kryo_core::ipc::{Completion, Endpoint, IpcError, Message};
use kryo_core::mm::frame::{
    FRAME_SIZE, FramePool, MemoryRegion, PhysRange, RegionKind, normalize_memory,
};
use kryo_core::mm::vm::{AddressSpace, Permissions, PrivateError, PrivateVm, VmBackend, VmError};
use kryo_core::object::{ObjectError, ObjectKind, ObjectTable};
use kryo_core::process::{ProcessTable, ProcessTableError};
use kryo_core::resource::{Budget, Limits, Resource};
use kryo_core::sync::{WaitError, WaitTable, WordWaitError, WordWaiters};
use kryo_core::task::{TaskError, TaskTable};

thread_local! {
    static FAIL: Cell<bool> = const { Cell::new(false) };
}

struct Allocator;

// Failure is local to the current test thread. Every other allocation and every
// deallocation uses the host allocator, including the test harness itself.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if FAIL.try_with(Cell::get).unwrap_or(false) {
            std::ptr::null_mut()
        } else {
            unsafe { System.alloc(layout) }
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if FAIL.try_with(Cell::get).unwrap_or(false) {
            std::ptr::null_mut()
        } else {
            unsafe { System.realloc(pointer, layout, size) }
        }
    }
}

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

fn fail_allocations<T>(operation: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            FAIL.with(|flag| flag.set(false));
        }
    }
    FAIL.with(|flag| flag.set(true));
    let _reset = Reset;
    operation()
}

#[test]
fn budgeted_metadata_oom_refunds_charges_before_retry() {
    let budget = Budget::new(Limits {
        shared_pages: 0,
        private_pages: 0,
        reserved_pages: 0,
        handles: 1,
        waits: 1,
        threads: 1,
        mappings: 0,
        tables: 0,
    });
    let objects = ObjectTable::<(), 1>::new();
    let object = objects.create(ObjectKind::Memory, ()).unwrap();
    let mut handles = HandleTable::<(), 1, 0>::budgeted(budget.clone());
    let failure = fail_allocations(|| handles.insert(object, Rights::READ)).unwrap_err();
    assert_eq!(failure.error, CapabilityError::Capacity);
    assert_eq!(failure.object.reference_count(), 1);
    assert_eq!(budget.count(Resource::Handles), 0);
    let handle = handles.insert(failure.object, Rights::READ).unwrap();
    assert_eq!(budget.count(Resource::Handles), 1);
    handles.close(handle).unwrap();
    assert_eq!(budget.count(Resource::Handles), 0);

    let mut tasks = TaskTable::<(), 1>::new();
    let task = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<(), 0>::growing();
    assert_eq!(
        fail_allocations(|| waits.begin_budgeted(task, None, &budget)),
        Err(WaitError::Capacity)
    );
    assert_eq!(budget.count(Resource::Waits), 0);
    let wait = waits.begin_budgeted(task, None, &budget).unwrap();
    assert_eq!(budget.count(Resource::Waits), 1);
    waits.abandon(wait).unwrap();
    assert_eq!(budget.count(Resource::Waits), 0);
}

#[test]
fn task_and_process_oom_return_the_exact_unpublished_owner() {
    let mut tasks = TaskTable::<String, 0>::growing();
    let owner = String::from("thread owner");
    let pointer = owner.as_ptr();
    let failure = fail_allocations(|| tasks.spawn(owner)).unwrap_err();
    assert_eq!(failure.error, TaskError::Capacity);
    assert_eq!(failure.resources.as_ptr(), pointer);
    assert_eq!(tasks.live_count(), 0);
    tasks.spawn(failure.resources).unwrap();

    let mut processes = ProcessTable::<String, 0, 0>::growing();
    let owner = String::from("process owner");
    let pointer = owner.as_ptr();
    let failure = fail_allocations(|| processes.insert(owner)).unwrap_err();
    assert_eq!(failure.error, ProcessTableError::Capacity);
    assert_eq!(failure.resources.as_ptr(), pointer);
    let process = processes.insert(failure.resources).unwrap();
    let mut members = TaskTable::<(), 1>::new();
    let member = members.spawn(()).unwrap();
    assert_eq!(
        fail_allocations(|| processes.attach(process, member)),
        Err(ProcessTableError::ThreadCapacity)
    );
    assert_eq!(processes.thread_count(process), Ok(0));
    processes.attach(process, member).unwrap();
}

#[test]
fn object_and_handle_oom_preserve_strong_ownership_and_allow_retry() {
    let arena = ObjectTable::<String, 0>::growing();
    let value = String::from("payload");
    let pointer = value.as_ptr();
    let failure = fail_allocations(|| arena.create(ObjectKind::Image, value)).unwrap_err();
    assert_eq!(failure.error, ObjectError::Capacity);
    assert_eq!(failure.value.as_ptr(), pointer);
    assert_eq!(arena.live_count(), 0);
    let object = arena.create(ObjectKind::Image, failure.value).unwrap();
    let id = object.id();
    let mut handles = HandleTable::<String, 0, 0>::growing();
    let failure = fail_allocations(|| handles.insert(object, Rights::READ)).unwrap_err();
    assert_eq!(failure.error, CapabilityError::Capacity);
    assert_eq!(failure.object.id(), id);
    assert_eq!(failure.object.reference_count(), 1);
    assert_eq!(handles.live_count(), 0);
    assert_eq!(
        fail_allocations(|| handles.reserve(2)).unwrap_err(),
        CapabilityError::Capacity
    );
    assert_eq!(handles.reserved_count(), 0);
    let handle = handles.insert(failure.object, Rights::READ).unwrap();
    handles.close(handle).unwrap();
    assert_eq!(arena.live_count(), 0);
}

#[test]
fn wait_and_endpoint_oom_do_not_register_or_accept_a_partial_request() {
    let mut tasks = TaskTable::<(), 2>::new();
    let owner = tasks.spawn(()).unwrap();
    let sender = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<Completion, 0>::growing();
    assert_eq!(
        fail_allocations(|| waits.begin(sender)),
        Err(WaitError::Capacity)
    );
    assert_eq!(waits.live_count(), 0);
    let wait = waits.begin(sender).unwrap();
    let mut endpoint = Endpoint::<0, 0>::growing(owner);
    assert_eq!(
        fail_allocations(|| endpoint.call(sender, wait, Message::default(), &mut waits)),
        Err(IpcError::Capacity)
    );
    assert!(waits.is_pending(wait));
    endpoint
        .call(sender, wait, Message::default(), &mut waits)
        .unwrap();
    endpoint.close(&mut waits);
    waits.park(wait).unwrap();
    assert!(matches!(
        waits.take(wait),
        Ok(Completion::Failed {
            error: IpcError::Closed,
            ..
        })
    ));
}

struct FixedVm {
    root: u64,
    mappings: [Option<(u64, u64)>; 32],
}

// The model never dereferences logical physical addresses or allocates metadata.
unsafe impl VmBackend for FixedVm {
    type Error = ();
    fn root_frame(&self) -> u64 {
        self.root
    }
    fn check_inactive(&self) -> Result<(), ()> {
        Ok(())
    }
    fn tables_needed(&self, _: u64) -> Result<usize, ()> {
        Ok(0)
    }
    unsafe fn initialize_frame(&mut self, _: u64, _: &[u8]) -> Result<(), ()> {
        Ok(())
    }
    unsafe fn seal_frame(&mut self, _: u64, _: Permissions) -> Result<(), ()> {
        Ok(())
    }
    unsafe fn restore_frame(&mut self, _: u64) -> Result<(), ()> {
        Ok(())
    }
    unsafe fn map(
        &mut self,
        address: u64,
        physical: u64,
        _: Permissions,
        _: &[u64],
    ) -> Result<usize, ()> {
        let slot = self
            .mappings
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(())?;
        *slot = Some((address, physical));
        Ok(0)
    }
    unsafe fn unmap(&mut self, address: u64) -> Result<u64, ()> {
        let slot = self
            .mappings
            .iter_mut()
            .find(|slot| slot.is_some_and(|entry| entry.0 == address))
            .ok_or(())?;
        Ok(slot.take().unwrap().1)
    }
    unsafe fn read_frame(&self, _: u64, _: usize, output: &mut [u8]) -> Result<(), ()> {
        output.fill(0);
        Ok(())
    }
    unsafe fn retire(&mut self) -> Result<(), ()> {
        assert!(self.mappings.iter().all(Option::is_none));
        Ok(())
    }
}

#[test]
fn private_vm_allocator_null_preserves_reservations_owners_and_quota() {
    let pool = FramePool::<1, 4, 1>::new();
    let mut regions = [MemoryRegion {
        range: PhysRange::from_base_len(0x1000, 128 * FRAME_SIZE).unwrap(),
        kind: RegionKind::Usable,
    }];
    let mut output = [PhysRange::EMPTY; 1];
    let plan = normalize_memory(&mut regions, &mut [], &mut output).unwrap();
    unsafe { pool.initialize(plan) }.unwrap();
    let before = pool.stats();
    let budget = Budget::new(Limits {
        shared_pages: 0,
        private_pages: 32,
        reserved_pages: 32,
        handles: 0,
        waits: 0,
        threads: 0,
        mappings: 32,
        tables: 16,
    });
    let mut space = AddressSpace::<_, _, 0, 0>::new_limited(
        &pool,
        |root| {
            Ok(FixedVm {
                root,
                mappings: [None; 32],
            })
        },
        32,
        16,
    )
    .unwrap();
    let mut memory = PrivateVm::new();
    let root_only = pool.stats();
    assert_eq!(
        fail_allocations(|| memory.reserve(&space, &budget, 32 * FRAME_SIZE, FRAME_SIZE)),
        Err(PrivateError::OutOfResources),
    );
    assert_eq!(budget.count(Resource::ReservedPages), 0);
    assert_eq!(pool.stats(), root_only);
    let base = memory
        .reserve(&space, &budget, 32 * FRAME_SIZE, FRAME_SIZE)
        .unwrap();
    // The first commit needs its interval vector before acquiring backing.
    assert_eq!(
        fail_allocations(|| memory.commit(&mut space, base, FRAME_SIZE)),
        Err(PrivateError::OutOfResources),
    );
    assert_eq!(budget.count(Resource::PrivatePages), 0);
    assert_eq!(space.mapped_pages(), 0);
    assert_eq!(pool.stats(), root_only);
    memory.commit(&mut space, base, FRAME_SIZE).unwrap();
    let one_page = pool.stats();
    // Interval capacity is reusable; a larger commit now needs mapping metadata.
    assert_eq!(
        fail_allocations(|| memory.commit(&mut space, base + FRAME_SIZE, 8 * FRAME_SIZE)),
        Err(PrivateError::Vm(VmError::MappingCapacity)),
    );
    assert_eq!(space.mapped_pages(), 1);
    assert_eq!(budget.count(Resource::PrivatePages), 1);
    assert_eq!(pool.stats(), one_page);
    assert!(!memory.retirement_pending());
    memory
        .commit(&mut space, base + FRAME_SIZE, 8 * FRAME_SIZE)
        .unwrap();
    memory.release(&mut space, base).unwrap();
    assert_eq!(budget.count(Resource::PrivatePages), 0);
    assert_eq!(budget.count(Resource::ReservedPages), 0);
    space.close().unwrap();
    memory.clear_after_close(&space).unwrap();
    assert_eq!(pool.stats(), before);
}

#[test]
fn private_decommit_metadata_oom_does_not_invalidate_a_live_word() {
    let pool = FramePool::<1, 4, 1>::new();
    let mut regions = [MemoryRegion {
        range: PhysRange::from_base_len(0x1000, 128 * FRAME_SIZE).unwrap(),
        kind: RegionKind::Usable,
    }];
    let mut output = [PhysRange::EMPTY; 1];
    let plan = normalize_memory(&mut regions, &mut [], &mut output).unwrap();
    unsafe { pool.initialize(plan) }.unwrap();
    let budget = Budget::new(Limits {
        shared_pages: 0,
        private_pages: 16,
        reserved_pages: 16,
        handles: 0,
        waits: 0,
        threads: 0,
        mappings: 16,
        tables: 16,
    });
    let mut space = AddressSpace::<_, _, 0, 0>::new_limited(
        &pool,
        |root| {
            Ok(FixedVm {
                root,
                mappings: [None; 32],
            })
        },
        16,
        16,
    )
    .unwrap();
    let mut memory = PrivateVm::new();
    let base = memory
        .reserve(&space, &budget, 16 * FRAME_SIZE, FRAME_SIZE)
        .unwrap();
    memory.commit(&mut space, base, 3 * FRAME_SIZE).unwrap();
    for offset in [4, 6, 8] {
        memory
            .commit(&mut space, base + offset * FRAME_SIZE, FRAME_SIZE)
            .unwrap();
    }
    let address = base + FRAME_SIZE;
    let key = space.private_word_key(address).unwrap();
    let before = pool.stats();
    let invalidated = Cell::new(false);
    assert_eq!(
        fail_allocations(|| memory
            .decommit_with(&mut space, address, FRAME_SIZE, |_, _| invalidated
                .set(true))),
        Err(PrivateError::OutOfResources),
    );
    assert!(!invalidated.get());
    assert_eq!(space.private_word_key(address), Ok(key));
    assert_eq!(space.mapped_pages(), 6);
    assert_eq!(budget.count(Resource::PrivatePages), 6);
    assert_eq!(budget.count(Resource::ReservedPages), 16);
    assert_eq!(pool.stats(), before);
    memory
        .decommit_with(&mut space, address, FRAME_SIZE, |_, _| {
            invalidated.set(true)
        })
        .unwrap();
    assert!(invalidated.get());
    assert_eq!(space.private_word_key(address), Err(VmError::NotMapped));
    assert_eq!(budget.count(Resource::PrivatePages), 5);
    memory.release(&mut space, base).unwrap();
    space.close().unwrap();
}

#[test]
fn word_queue_oom_preserves_admitted_waits_and_prepared_paths_do_not_allocate() {
    let budget = Budget::new(Limits {
        shared_pages: 0,
        private_pages: 0,
        reserved_pages: 0,
        handles: 0,
        waits: 2,
        threads: 0,
        mappings: 0,
        tables: 0,
    });
    let mut tasks = TaskTable::<(), 2>::new();
    let first = tasks.spawn(()).unwrap();
    let second = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<u8, 2>::new();
    let mut words = WordWaiters::<u64>::new();
    assert_eq!(
        fail_allocations(|| words.prepare(1)),
        Err(WordWaitError::Capacity)
    );
    assert!(words.is_empty());
    assert_eq!(waits.live_count(), 0);
    assert_eq!(budget.count(Resource::Waits), 0);

    words.prepare(1).unwrap();
    let first = waits.begin_budgeted(first, None, &budget).unwrap();
    fail_allocations(|| words.push(7, first)).unwrap();
    assert_eq!(
        fail_allocations(|| words.prepare(1024)),
        Err(WordWaitError::Capacity)
    );
    assert_eq!(words.len(), 1);
    assert!(waits.is_pending(first));
    assert_eq!(budget.count(Resource::Waits), 1);
    words.prepare(2).unwrap();
    let second = waits.begin_budgeted(second, None, &budget).unwrap();
    words.push(7, second).unwrap();
    waits.park(first).unwrap();
    waits.park(second).unwrap();

    let completed = fail_allocations(|| {
        words.remove_matching(
            |_| true,
            |wait| {
                waits.complete(wait, 9).unwrap();
            },
        )
    });
    assert_eq!(completed, 2);
    assert!(words.is_empty());
    assert_eq!(budget.count(Resource::Waits), 2);
    assert_eq!(waits.take(first), Ok(9));
    assert_eq!(waits.take(second), Ok(9));
    assert_eq!(budget.count(Resource::Waits), 0);
}
