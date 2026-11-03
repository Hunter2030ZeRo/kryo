use kryo_core::sync::{ParkResult, WaitError, WaitTable, WordWaitError, WordWaiters};
use kryo_core::task::TaskTable;

#[test]
fn bounded_wake_preserves_matching_fifo_and_other_queues() {
    let mut tasks = TaskTable::<(), 5>::new();
    let ids = core::array::from_fn::<_, 5, _>(|_| tasks.spawn(()).unwrap());
    let mut waits = WaitTable::<u8, 5>::new();
    let registered = ids.map(|task| waits.begin(task).unwrap());
    let mut queue = WordWaiters::new();
    queue.prepare(registered.len()).unwrap();
    for (key, wait) in [10, 20, 10, 10, 20].into_iter().zip(registered) {
        queue.push(key, wait).unwrap();
        waits.park(wait).unwrap();
    }

    let mut order = Vec::new();
    assert_eq!(
        queue.wake(10, 2, |wait| {
            order.push(wait);
            waits.complete(wait, 1).unwrap();
        }),
        2
    );
    assert_eq!(order, [registered[0], registered[2]]);
    assert_eq!(queue.len(), 3);
    assert_eq!(waits.take(registered[0]), Ok(1));
    assert_eq!(waits.take(registered[2]), Ok(1));
    for index in [1, 3, 4] {
        assert!(waits.is_pending(registered[index]));
    }

    order.clear();
    assert_eq!(queue.wake(20, usize::MAX, |wait| order.push(wait)), 2);
    assert_eq!(order, [registered[1], registered[4]]);
    assert_eq!(queue.wake(10, 1, |wait| order.push(wait)), 1);
    assert_eq!(order, [registered[1], registered[4], registered[3]]);
    assert!(queue.is_empty());
    for wait in &order {
        waits.complete(*wait, 2).unwrap();
        assert_eq!(waits.take(*wait), Ok(2));
    }
}

#[test]
fn wake_does_not_latch_and_zero_limit_preserves_the_wait() {
    let mut tasks = TaskTable::<(), 1>::new();
    let task = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<(), 1>::new();
    let mut queue = WordWaiters::new();
    assert_eq!(queue.wake(7, 1, |_| panic!("empty wake")), 0);
    queue.prepare(1).unwrap();
    let wait = waits.begin(task).unwrap();
    queue.push(7, wait).unwrap();
    assert_eq!(waits.park(wait), Ok(ParkResult::Blocked));
    assert_eq!(queue.wake(7, 0, |_| panic!("zero wake limit")), 0);
    assert_eq!(queue.len(), 1);
    assert!(waits.is_pending(wait));
    assert_eq!(
        queue.wake(7, 1, |id| {
            waits.complete(id, ()).unwrap();
        }),
        1
    );
    assert_eq!(waits.take(wait), Ok(()));
}

#[test]
fn process_and_mapping_lifetimes_separate_equal_addresses() {
    let mut tasks = TaskTable::<(), 4>::new();
    let ids = core::array::from_fn::<_, 4, _>(|_| tasks.spawn(()).unwrap());
    let mut waits = WaitTable::<(), 4>::new();
    let registered = ids.map(|task| waits.begin(task).unwrap());
    let mut queue = WordWaiters::new();
    queue.prepare(4).unwrap();
    // Opaque production keys come from VM validation. These model keys vary
    // address-space identity, mapping lifetime, then word address independently.
    let keys = [
        (1, 1, 0x1000),
        (2, 1, 0x1000),
        (1, 2, 0x1000),
        (1, 1, 0x1004),
    ];
    for (key, wait) in keys.into_iter().zip(registered) {
        queue.push(key, wait).unwrap();
        waits.park(wait).unwrap();
    }
    assert_eq!(
        queue.wake(keys[2], usize::MAX, |id| {
            waits.complete(id, ()).unwrap();
        }),
        1
    );
    assert_eq!(waits.take(registered[2]), Ok(()));
    for index in [0, 1, 3] {
        assert!(waits.is_pending(registered[index]));
    }
    let mut removed = Vec::new();
    assert_eq!(
        queue.remove_matching(|key| key.0 == 1, |id| removed.push(id)),
        2
    );
    assert_eq!(removed, [registered[0], registered[3]]);
    assert_eq!(queue.len(), 1);
    assert!(queue.remove(registered[1]));
    assert!(queue.is_empty());
    for index in [0, 1, 3] {
        waits.abandon(registered[index]).unwrap();
    }
}

#[test]
fn detach_before_timeout_or_cancel_prevents_a_second_completion_after_reuse() {
    let mut tasks = TaskTable::<(), 1>::new();
    let task = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<u8, 1>::new();
    let mut queue = WordWaiters::new();
    queue.prepare(1).unwrap();
    let old = waits.begin_until(task, 100).unwrap();
    queue.push(3, old).unwrap();
    waits.park(old).unwrap();
    assert_eq!(waits.next_expired(100), Some(old));
    assert!(queue.remove(old));
    waits.complete(old, 9).unwrap();
    assert_eq!(queue.wake(3, 1, |_| panic!("detached timeout")), 0);
    assert_eq!(waits.take(old), Ok(9));

    let next = waits.begin(task).unwrap();
    queue.push(3, next).unwrap();
    assert_ne!(old, next);
    assert!(!queue.remove(old));
    assert_eq!(waits.complete(old, 8), Err(WaitError::InvalidId));
    assert_eq!(
        queue.remove_matching(
            |key| key == 3,
            |id| {
                waits.complete(id, 7).unwrap();
            }
        ),
        1
    );
    assert_eq!(waits.next_ready(), None);
    assert_eq!(waits.park(next), Ok(ParkResult::Ready));
    assert_eq!(waits.complete(next, 8), Err(WaitError::AlreadyCompleted));
    assert_eq!(waits.take(next), Ok(7));
}

#[test]
fn admission_rejects_unprepared_or_duplicate_ids_without_losing_queued_waits() {
    let mut tasks = TaskTable::<(), 1>::new();
    let task = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<(), 1>::new();
    let wait = waits.begin(task).unwrap();
    let mut queue = WordWaiters::new();
    assert_eq!(queue.push(1, wait), Err(WordWaitError::Capacity));
    assert!(waits.is_pending(wait));
    queue.prepare(1).unwrap();
    queue.push(1, wait).unwrap();
    assert_eq!(queue.push(2, wait), Err(WordWaitError::DuplicateWait));
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.wake(2, 1, |_| panic!("wrong key")), 0);
    assert!(queue.remove(wait));
    assert!(!queue.remove(wait));
    assert_eq!(waits.abandon(wait), Ok(None));
}
