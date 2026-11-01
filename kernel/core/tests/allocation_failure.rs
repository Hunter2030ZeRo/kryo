use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use kryo_core::capability::{CapabilityError, HandleTable, Rights};
use kryo_core::ipc::{Completion, Endpoint, IpcError, Message};
use kryo_core::object::{ObjectError, ObjectKind, ObjectTable};
use kryo_core::process::{ProcessTable, ProcessTableError};
use kryo_core::sync::{WaitError, WaitTable};
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
