use core::cell::RefCell;
use kryo_core::capability::{HandleTable, Rights};
use kryo_core::ipc::{Completion, Delivery, IpcError};
use kryo_core::object::{ObjectKind, ObjectTable};
use kryo_core::process::{ExitReason, ExitStatus, ProcessError, ProcessExit};
use kryo_core::sched::TaskState;
use kryo_core::sync::{ParkResult, WaitId, WaitTable};
use kryo_core::task::{ReapError, TaskError, TaskId, TaskTable};

type Waits = WaitTable<Completion, 3>;

const EXIT: ExitStatus = ExitStatus {
    reason: ExitReason::Exited,
    code: 41,
};
const FAULT: ExitStatus = ExitStatus {
    reason: ExitReason::Fault,
    code: 14,
};

#[test]
fn an_exit_object_binds_once_before_publication() {
    let (_tasks, [target, other, _, _]) = tasks();
    let mut process = ProcessExit::<2>::new();
    let mut waits = Waits::new();
    assert_eq!(process.task(), None);
    assert_eq!(process.result(), None);
    assert_eq!(process.finish(EXIT, &mut waits), Err(ProcessError::Unbound));
    let wait = waits.begin(other).unwrap();
    assert_eq!(
        process.wait(other, wait, &mut waits),
        Err(IpcError::InvalidWait)
    );
    assert!(waits.is_pending(wait));
    process.bind(target).unwrap();
    assert_eq!(process.task(), Some(target));
    assert_eq!(process.bind(other), Err(ProcessError::AlreadyBound));
    assert_eq!(process.task(), Some(target));
    assert_eq!(process.result(), None);
}

#[test]
fn observers_before_and_after_exit_receive_the_same_durable_result() {
    for parking in [false, true] {
        let (_tasks, [target, first, second, third]) = tasks();
        let mut process = bound::<2>(target);
        let mut waits = Waits::new();
        let first_wait = waits.begin_until(first, 100).unwrap();
        let second_wait = waits.begin_until(second, 200).unwrap();
        if !parking {
            waits.park(first_wait).unwrap();
            waits.park(second_wait).unwrap();
        }
        process.wait(first, first_wait, &mut waits).unwrap();
        process.wait(second, second_wait, &mut waits).unwrap();
        process.finish(EXIT, &mut waits).unwrap();
        assert_eq!(process.result(), Some(EXIT));
        assert_eq!(waits.next_deadline(), None);
        if parking {
            assert_eq!(waits.next_ready(), None);
            assert_eq!(waits.park(first_wait), Ok(ParkResult::Ready));
            assert_eq!(waits.park(second_wait), Ok(ParkResult::Ready));
        }
        assert_eq!(waits.take(first_wait), Ok(Completion::ProcessExited(EXIT)));
        assert_eq!(waits.take(second_wait), Ok(Completion::ProcessExited(EXIT)));
        let late = blocked(&mut waits, third);
        process.wait(third, late, &mut waits).unwrap();
        assert_eq!(waits.take(late), Ok(Completion::ProcessExited(EXIT)));
        assert_eq!(
            process.finish(FAULT, &mut waits),
            Err(ProcessError::AlreadyExited)
        );
        assert_eq!(process.result(), Some(EXIT));
    }
}

#[test]
fn self_wait_and_foreign_or_completed_wait_ids_are_rejected() {
    let (_tasks, [target, observer, stranger, _]) = tasks();
    let mut process = bound::<1>(target);
    let mut waits = Waits::new();
    let own = blocked(&mut waits, target);
    assert_eq!(
        process.wait(target, own, &mut waits),
        Err(IpcError::PermissionDenied)
    );
    let other = blocked(&mut waits, observer);
    assert_eq!(
        process.wait(stranger, other, &mut waits),
        Err(IpcError::InvalidWait)
    );
    waits.complete(other, Completion::Slept).unwrap();
    assert_eq!(
        process.wait(observer, other, &mut waits),
        Err(IpcError::InvalidWait)
    );
    process.finish(EXIT, &mut waits).unwrap();
    assert!(waits.is_pending(own));
    assert_eq!(waits.take(other), Ok(Completion::Slept));
}

#[test]
fn observer_capacity_failure_preserves_existing_waits_and_recovers_after_cancel() {
    let (_tasks, [target, first, second, _]) = tasks();
    let mut process = bound::<1>(target);
    let mut waits = Waits::new();
    let first_wait = blocked(&mut waits, first);
    let second_wait = blocked(&mut waits, second);
    process.wait(first, first_wait, &mut waits).unwrap();
    assert_eq!(
        process.wait(first, first_wait, &mut waits),
        Err(IpcError::AlreadyWaiting)
    );
    assert_eq!(
        process.wait(second, second_wait, &mut waits),
        Err(IpcError::Capacity)
    );
    assert!(waits.is_pending(first_wait));
    assert!(waits.is_pending(second_wait));
    process.cancel(first_wait, &mut waits).unwrap();
    process.wait(second, second_wait, &mut waits).unwrap();
    process.finish(EXIT, &mut waits).unwrap();
    assert_eq!(waits.take(first_wait), Ok(failed(IpcError::Cancelled)));
    assert_eq!(waits.take(second_wait), Ok(Completion::ProcessExited(EXIT)));
}

#[test]
fn terminal_observation_needs_no_pending_observer_capacity() {
    let (_tasks, [target, observer, _, _]) = tasks();
    let mut process = bound::<0>(target);
    let mut waits = Waits::new();
    let wait = blocked(&mut waits, observer);
    assert_eq!(
        process.wait(observer, wait, &mut waits),
        Err(IpcError::Capacity)
    );
    process.finish(EXIT, &mut waits).unwrap();
    process.wait(observer, wait, &mut waits).unwrap();
    assert_eq!(waits.take(wait), Ok(Completion::ProcessExited(EXIT)));
}

#[test]
fn cancellation_and_timeout_detach_only_the_selected_observer() {
    for timeout in [false, true] {
        let (_tasks, [target, first, second, _]) = tasks();
        let mut process = bound::<2>(target);
        let mut waits = Waits::new();
        let first_wait = blocked(&mut waits, first);
        let second_wait = blocked(&mut waits, second);
        process.wait(first, first_wait, &mut waits).unwrap();
        process.wait(second, second_wait, &mut waits).unwrap();
        let error = if timeout {
            process.timeout(first_wait, &mut waits).unwrap();
            IpcError::TimedOut
        } else {
            process.cancel(first_wait, &mut waits).unwrap();
            IpcError::Cancelled
        };
        assert_eq!(process.result(), None);
        process.finish(FAULT, &mut waits).unwrap();
        assert_eq!(waits.take(first_wait), Ok(failed(error)));
        assert_eq!(
            waits.take(second_wait),
            Ok(Completion::ProcessExited(FAULT))
        );
        assert_eq!(process.result(), Some(FAULT));
    }
}

#[test]
fn exit_that_wins_cannot_be_overwritten_by_timeout_or_cancel() {
    let (_tasks, [target, observer, _, _]) = tasks();
    let mut process = bound::<1>(target);
    let mut waits = Waits::new();
    let wait = blocked(&mut waits, observer);
    process.wait(observer, wait, &mut waits).unwrap();
    process.finish(EXIT, &mut waits).unwrap();
    assert_eq!(
        process.timeout(wait, &mut waits),
        Err(IpcError::InvalidWait)
    );
    assert_eq!(process.cancel(wait, &mut waits), Err(IpcError::InvalidWait));
    assert_eq!(waits.take(wait), Ok(Completion::ProcessExited(EXIT)));
}

#[test]
fn observer_death_does_not_terminate_target_or_retain_a_stale_wait() {
    let (mut tasks, [target, observer, _, _]) = tasks();
    let mut process = bound::<1>(target);
    let mut waits = Waits::new();
    let old = blocked(&mut waits, observer);
    process.wait(observer, old, &mut waits).unwrap();
    tasks.stop(observer).unwrap();
    process.task_died(observer, &mut waits);
    assert_eq!(waits.abandon(old), Ok(Some(failed(IpcError::PeerClosed))));
    tasks.reap(observer, |_| Ok::<_, ()>(())).unwrap();
    let replacement = tasks.spawn(()).unwrap();
    assert_ne!(replacement, observer);
    let new = blocked(&mut waits, replacement);
    assert_ne!(new, old);
    process.wait(replacement, new, &mut waits).unwrap();
    assert_eq!(process.cancel(old, &mut waits), Err(IpcError::InvalidWait));
    process.task_died(observer, &mut waits);
    assert!(waits.is_pending(new));
    assert_eq!(tasks.state(target), Ok(TaskState::Runnable));
    assert_eq!(process.result(), None);
    process.finish(EXIT, &mut waits).unwrap();
    assert_eq!(waits.take(new), Ok(Completion::ProcessExited(EXIT)));
}

#[test]
fn task_cleanup_retry_and_slot_reuse_do_not_change_old_control_result() {
    let mut tasks = TaskTable::<u8, 1>::new();
    let old_task = tasks.spawn(7).unwrap();
    let mut old = bound::<0>(old_task);
    let mut waits = Waits::new();
    tasks.stop(old_task).unwrap();
    old.finish(EXIT, &mut waits).unwrap();
    assert_eq!(
        tasks.reap(old_task, |_| Err("cleanup pending")),
        Err(ReapError::Cleanup("cleanup pending"))
    );
    assert_eq!(tasks.state(old_task), Ok(TaskState::Dying));
    assert_eq!(old.result(), Some(EXIT));
    assert_eq!(tasks.resources_mut(old_task), Ok(&mut 7));
    tasks.reap(old_task, |_| Ok::<_, ()>(())).unwrap();
    let replacement = tasks.spawn(8).unwrap();
    assert_ne!(replacement, old_task);
    assert_eq!(tasks.state(old_task), Err(TaskError::InvalidId));
    let mut current = bound::<0>(replacement);
    assert_eq!(current.result(), None);
    assert_eq!(old.task(), Some(old_task));
    assert_eq!(old.result(), Some(EXIT));
    let stopped = ExitStatus {
        reason: ExitReason::Stopped,
        code: 9,
    };
    current.finish(stopped, &mut waits).unwrap();
    assert_eq!(current.result(), Some(stopped));
    assert_eq!(old.result(), Some(EXIT));
}

#[test]
fn retained_control_reference_survives_handle_close_and_task_reap() {
    let mut tasks = TaskTable::<(), 1>::new();
    let target = tasks.spawn(()).unwrap();
    let mut waits = Waits::new();
    let objects = ObjectTable::<RefCell<ProcessExit<0>>, 1>::new();
    let object = objects
        .create(ObjectKind::ProcessControl, RefCell::new(bound(target)))
        .unwrap();
    let mut handles = HandleTable::<RefCell<ProcessExit<0>>, 1, 1>::new();
    let handle = handles
        .insert(object.try_clone().unwrap(), Rights::WAIT | Rights::INSPECT)
        .unwrap();
    let retained = handles
        .lookup(handle, ObjectKind::ProcessControl, Rights::INSPECT)
        .unwrap();
    tasks.stop(target).unwrap();
    object
        .with(|process| process.borrow_mut().finish(EXIT, &mut waits))
        .unwrap();
    tasks.reap(target, |_| Ok::<_, ()>(())).unwrap();
    handles.close(handle).unwrap();
    drop(object);
    assert_eq!(objects.live_count(), 1);
    assert_eq!(
        retained.with(|process| process.borrow().result()),
        Some(EXIT)
    );
    drop(retained);
    assert_eq!(objects.live_count(), 0);
}

fn failed(error: IpcError) -> Completion {
    Completion::Failed {
        error,
        delivery: Delivery::NotDelivered,
    }
}

fn bound<const OBSERVERS: usize>(task: TaskId) -> ProcessExit<OBSERVERS> {
    let mut process = ProcessExit::new();
    process.bind(task).unwrap();
    process
}

fn blocked(waits: &mut Waits, task: TaskId) -> WaitId {
    let wait = waits.begin_until(task, 100).unwrap();
    assert_eq!(waits.park(wait), Ok(ParkResult::Blocked));
    wait
}

fn tasks() -> (TaskTable<(), 4>, [TaskId; 4]) {
    let mut tasks = TaskTable::new();
    let ids = core::array::from_fn(|_| tasks.spawn(()).unwrap());
    (tasks, ids)
}
