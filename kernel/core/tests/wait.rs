use kryo_core::sched::TaskState;
use kryo_core::sync::{ParkResult, WaitError, WaitTable};
use kryo_core::task::TaskTable;

#[test]
fn early_completion_waits_for_execution_handoff() {
    let mut tasks = TaskTable::<(), 1>::new();
    let task = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<u64, 1>::new();
    let id = waits.begin(task).unwrap();
    assert_eq!(waits.task(id), Ok(task));
    assert!(waits.is_pending(id));

    assert_eq!(waits.complete(id, 42), Ok(None));
    assert!(!waits.is_pending(id));
    assert_eq!(waits.next_ready(), None);
    assert_eq!(waits.take(id), Err(WaitError::NotReady));
    assert_eq!(waits.complete(id, 99), Err(WaitError::AlreadyCompleted));

    assert_eq!(waits.park(id), Ok(ParkResult::Ready));
    assert_eq!(waits.next_ready(), Some((id, task)));
    assert_eq!(waits.take(id), Ok(42));
    assert_eq!(waits.task(id), Err(WaitError::InvalidId));
}

#[test]
fn late_completion_returns_the_blocked_task_once() {
    let mut tasks = TaskTable::<(), 1>::new();
    let task = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<u8, 1>::new();
    let id = waits.begin(task).unwrap();
    assert_eq!(waits.park(id), Ok(ParkResult::Blocked));
    assert_eq!(waits.park(id), Err(WaitError::AlreadyParked));
    assert_eq!(waits.take(id), Err(WaitError::NotReady));

    assert_eq!(waits.complete(id, 7), Ok(Some(task)));
    assert_eq!(waits.complete(id, 8), Err(WaitError::AlreadyCompleted));
    assert_eq!(waits.next_ready(), Some((id, task)));
    assert_eq!(waits.take(id), Ok(7));
    assert_eq!(waits.take(id), Err(WaitError::InvalidId));
    assert_eq!(waits.complete(id, 9), Err(WaitError::InvalidId));
}

#[test]
fn a_task_has_one_wait_until_its_completion_is_consumed() {
    let mut tasks = TaskTable::<(), 1>::new();
    let task = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<(), 2>::new();
    let id = waits.begin(task).unwrap();
    assert_eq!(waits.begin(task), Err(WaitError::AlreadyWaiting));
    waits.complete(id, ()).unwrap();
    assert_eq!(waits.begin(task), Err(WaitError::AlreadyWaiting));
    waits.park(id).unwrap();
    assert_eq!(waits.begin(task), Err(WaitError::AlreadyWaiting));
    waits.take(id).unwrap();

    let next = waits.begin(task).unwrap();
    assert_ne!(next, id);
    assert_eq!(waits.complete(id, ()), Err(WaitError::InvalidId));
    assert!(waits.is_pending(next));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Completion {
    Signal,
    Cancel,
}

#[test]
fn signal_and_cancellation_order_selects_one_result() {
    let mut tasks = TaskTable::<(), 1>::new();
    let task = tasks.spawn(()).unwrap();
    for blocked in [false, true] {
        for (winner, loser) in [
            (Completion::Signal, Completion::Cancel),
            (Completion::Cancel, Completion::Signal),
        ] {
            let mut waits = WaitTable::<Completion, 1>::new();
            let id = waits.begin(task).unwrap();
            if blocked {
                waits.park(id).unwrap();
            }
            assert_eq!(waits.complete(id, winner), Ok(blocked.then_some(task)));
            assert_eq!(waits.complete(id, loser), Err(WaitError::AlreadyCompleted));
            if !blocked {
                assert_eq!(waits.next_ready(), None);
                assert_eq!(waits.park(id), Ok(ParkResult::Ready));
            }
            assert_eq!(waits.take(id), Ok(winner));
        }
    }
}

#[test]
fn ready_scan_delivers_each_completed_wait_once() {
    let mut tasks = TaskTable::<(), 3>::new();
    let ids = core::array::from_fn::<_, 3, _>(|_| tasks.spawn(()).unwrap());
    let mut waits = WaitTable::<usize, 3>::new();
    let waits_ids = ids.map(|task| waits.begin(task).unwrap());
    for id in waits_ids {
        waits.park(id).unwrap();
    }
    for index in [2, 0, 1] {
        assert_eq!(
            waits.complete(waits_ids[index], index),
            Ok(Some(ids[index]))
        );
    }
    let mut observed = [false; 3];
    while let Some((id, task)) = waits.next_ready() {
        let value = waits.take(id).unwrap();
        assert_eq!(ids[value], task);
        assert!(!observed[value]);
        observed[value] = true;
    }
    assert_eq!(observed, [true; 3]);
}

#[test]
fn abandon_returns_any_recorded_value_and_invalidates_the_wait() {
    let mut tasks = TaskTable::<(), 1>::new();
    let task = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<u32, 1>::new();

    let parking = waits.begin(task).unwrap();
    assert_eq!(waits.abandon(parking), Ok(None));
    let blocked = waits.begin(task).unwrap();
    waits.park(blocked).unwrap();
    assert_eq!(waits.abandon(blocked), Ok(None));

    let early = waits.begin(task).unwrap();
    waits.complete(early, 17).unwrap();
    assert_eq!(waits.abandon(early), Ok(Some(17)));
    let ready = waits.begin(task).unwrap();
    waits.park(ready).unwrap();
    waits.complete(ready, 19).unwrap();
    assert_eq!(waits.abandon(ready), Ok(Some(19)));

    for stale in [parking, blocked, early, ready] {
        assert_eq!(waits.task(stale), Err(WaitError::InvalidId));
        assert!(!waits.is_pending(stale));
    }
    assert_eq!(waits.next_ready(), None);
}

#[test]
fn capacity_failure_preserves_the_existing_wait() {
    let mut tasks = TaskTable::<(), 2>::new();
    let first = tasks.spawn(()).unwrap();
    let second = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<u8, 1>::new();
    let id = waits.begin(first).unwrap();
    assert_eq!(waits.begin(second), Err(WaitError::Capacity));
    assert!(waits.is_pending(id));
    assert_eq!(waits.task(id), Ok(first));
    waits.abandon(id).unwrap();
    assert!(waits.begin(second).is_ok());

    let mut empty = WaitTable::<(), 0>::new();
    assert_eq!(empty.begin(first), Err(WaitError::Capacity));
    assert_eq!(empty.park(id), Err(WaitError::InvalidId));
}

#[test]
fn late_completion_cannot_wake_a_reused_task_slot() {
    let mut tasks = TaskTable::<(), 1>::new();
    let old_task = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<(), 1>::new();
    let old_wait = waits.begin(old_task).unwrap();
    waits.park(old_wait).unwrap();

    // External queues are detached before this wait is abandoned and its task reaped.
    waits.abandon(old_wait).unwrap();
    tasks.stop(old_task).unwrap();
    tasks.reap(old_task, |_| Ok::<_, ()>(())).unwrap();
    let new_task = tasks.spawn(()).unwrap();
    let new_wait = waits.begin(new_task).unwrap();
    assert_ne!(new_task, old_task);
    assert_ne!(new_wait, old_wait);
    assert_eq!(waits.complete(old_wait, ()), Err(WaitError::InvalidId));
    assert!(waits.is_pending(new_wait));
    assert_eq!(tasks.state(new_task), Ok(TaskState::Runnable));
}

#[test]
fn completions_need_not_be_copyable() {
    let mut tasks = TaskTable::<(), 1>::new();
    let task = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<String, 1>::new();
    let id = waits.begin(task).unwrap();
    waits.complete(id, String::from("completed")).unwrap();
    waits.park(id).unwrap();
    assert_eq!(waits.take(id).unwrap(), "completed");
}
