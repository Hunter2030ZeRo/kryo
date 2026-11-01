use kryo_core::sync::{ParkResult, WaitError, WaitTable};
use kryo_core::task::{TaskId, TaskTable};

#[test]
fn deadlines_are_absolute_and_select_earliest_then_slot() {
    let (_tasks, [first, second, third]) = tasks();
    let mut waits = WaitTable::<u8, 3>::new();
    let first = waits.begin_until(first, 20).unwrap();
    let second = waits.begin_until(second, 10).unwrap();
    let third = waits.begin_until(third, 10).unwrap();
    assert_eq!(waits.next_deadline(), Some(10));
    assert_eq!(waits.next_expired(9), None);
    assert_eq!(waits.next_expired(10), Some(second));
    assert_eq!(waits.next_expired(100), Some(second));

    waits.complete(second, 2).unwrap();
    assert_eq!(waits.next_expired(10), Some(third));
    waits.complete(third, 3).unwrap();
    assert_eq!(waits.next_deadline(), Some(20));
    assert_eq!(waits.next_expired(19), None);
    assert_eq!(waits.next_expired(20), Some(first));
    waits.complete(first, 1).unwrap();
    assert_eq!(waits.next_deadline(), None);
    assert_eq!(waits.next_expired(u64::MAX), None);
}

#[test]
fn selection_does_not_complete_or_publish_a_parking_wait() {
    let (_tasks, [task, _, _]) = tasks();
    let mut waits = WaitTable::<u8, 1>::new();
    let wait = waits.begin_until(task, 0).unwrap();
    assert_eq!(waits.next_deadline(), Some(0));
    assert_eq!(waits.next_expired(0), Some(wait));
    assert_eq!(waits.next_expired(0), Some(wait));
    assert!(waits.is_pending(wait));
    assert_eq!(waits.next_ready(), None);

    assert_eq!(waits.complete(wait, 7), Ok(None));
    assert_eq!(waits.next_deadline(), None);
    assert_eq!(waits.next_expired(0), None);
    assert_eq!(waits.next_ready(), None);
    assert_eq!(waits.take(wait), Err(WaitError::NotReady));
    assert_eq!(waits.complete(wait, 9), Err(WaitError::AlreadyCompleted));
    assert_eq!(waits.park(wait), Ok(ParkResult::Ready));
    assert_eq!(waits.take(wait), Ok(7));
}

#[test]
fn untimed_waits_and_maximum_deadlines_need_no_sentinel() {
    let (_tasks, [first, second, third]) = tasks();
    let mut waits = WaitTable::<(), 3>::new();
    let untimed = waits.begin(first).unwrap();
    assert_eq!(waits.next_deadline(), None);
    let maximum = waits.begin_until(second, u64::MAX).unwrap();
    let zero = waits.begin_until(third, 0).unwrap();
    assert_eq!(waits.next_expired(0), Some(zero));
    waits.abandon(zero).unwrap();
    assert_eq!(waits.next_deadline(), Some(u64::MAX));
    assert_eq!(waits.next_expired(u64::MAX - 1), None);
    assert_eq!(waits.next_expired(u64::MAX), Some(maximum));
    waits.abandon(maximum).unwrap();
    assert_eq!(waits.next_expired(u64::MAX), None);
    assert!(waits.is_pending(untimed));
}

#[test]
fn failed_registration_preserves_deadline_and_capacity() {
    let (_tasks, [first, second, _]) = tasks();
    let mut waits = WaitTable::<(), 1>::new();
    let wait = waits.begin_until(first, 40).unwrap();
    assert_eq!(waits.begin_until(first, 5), Err(WaitError::AlreadyWaiting));
    assert_eq!(waits.begin_until(second, 5), Err(WaitError::Capacity));
    assert_eq!(waits.next_deadline(), Some(40));
    assert_eq!(waits.next_expired(5), None);
    assert_eq!(waits.next_expired(40), Some(wait));
    let mut empty = WaitTable::<(), 0>::new();
    assert_eq!(empty.begin_until(second, 0), Err(WaitError::Capacity));
    assert_eq!(empty.next_deadline(), None);
}

#[test]
fn taking_or_abandoning_removes_old_deadlines_before_slot_reuse() {
    let (_tasks, [task, _, _]) = tasks();
    let mut waits = WaitTable::<(), 1>::new();
    let old = waits.begin_until(task, 5).unwrap();
    waits.park(old).unwrap();
    waits.complete(old, ()).unwrap();
    assert_eq!(waits.next_deadline(), None);
    waits.take(old).unwrap();

    let abandoned = waits.begin_until(task, 6).unwrap();
    waits.abandon(abandoned).unwrap();
    assert_eq!(waits.next_deadline(), None);
    let current = waits.begin_until(task, 100).unwrap();
    assert_ne!(current, old);
    assert_ne!(current, abandoned);
    assert_eq!(waits.complete(old, ()), Err(WaitError::InvalidId));
    assert_eq!(waits.complete(abandoned, ()), Err(WaitError::InvalidId));
    assert_eq!(waits.next_expired(99), None);
    assert_eq!(waits.next_expired(100), Some(current));
}

fn tasks() -> (TaskTable<(), 3>, [TaskId; 3]) {
    let mut tasks = TaskTable::new();
    let ids = core::array::from_fn(|_| tasks.spawn(()).unwrap());
    (tasks, ids)
}
