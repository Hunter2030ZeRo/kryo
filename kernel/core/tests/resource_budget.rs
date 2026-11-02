use kryo_core::capability::{CapabilityError, HandleTable, Rights, TransferSpec};
use kryo_core::object::{ObjectKind, ObjectTable};
use kryo_core::resource::{Budget, BudgetError, Limits, Resource};
use kryo_core::sync::wait::{WaitError, WaitTable};
use kryo_core::task::TaskTable;

fn limits() -> Limits {
    Limits {
        shared_pages: 8,
        handles: 2,
        waits: 1,
        threads: 2,
        mappings: 16,
        tables: 8,
    }
}

#[test]
fn inherited_accounts_share_charges_and_independent_accounts_do_not() {
    let parent = Budget::new(limits());
    let child = parent.clone();
    let other = Budget::new(limits());
    let parent_thread = parent.try_charge(Resource::Threads, 1).unwrap();
    let child_thread = child.try_charge(Resource::Threads, 1).unwrap();
    assert_eq!(child.limits(), limits());
    assert!(matches!(
        parent.try_charge(Resource::Threads, 1),
        Err(BudgetError::Capacity)
    ));
    assert_eq!(other.count(Resource::Threads), 0);
    drop(parent);
    assert_eq!(child.count(Resource::Threads), 2);
    drop(parent_thread);
    assert_eq!(child.count(Resource::Threads), 1);
    drop(child_thread);
    assert_eq!(child.count(Resource::Threads), 0);
}

#[test]
fn multi_unit_charges_reject_zero_overflow_and_exhaustion_without_mutation() {
    let budget = Budget::new(Limits {
        shared_pages: usize::MAX,
        ..limits()
    });
    assert!(matches!(
        budget.try_charge(Resource::SharedPages, 0),
        Err(BudgetError::InvalidAmount)
    ));
    let charge = budget
        .try_charge(Resource::SharedPages, usize::MAX)
        .unwrap();
    assert_eq!(charge.amount(), usize::MAX);
    assert_eq!(charge.resource(), Resource::SharedPages);
    assert!(matches!(
        budget.try_charge(Resource::SharedPages, 1),
        Err(BudgetError::Capacity)
    ));
    assert_eq!(budget.count(Resource::SharedPages), usize::MAX);
    assert_eq!(budget.count(Resource::Handles), 0);
    drop(charge);
    assert_eq!(budget.count(Resource::SharedPages), 0);
}

#[test]
fn handle_duplication_and_transfer_share_account_and_charge_receiver_reservations() {
    let budget = Budget::new(limits());
    let objects = ObjectTable::<(), 1>::new();
    let object = objects.create(ObjectKind::Memory, ()).unwrap();
    let mut parent = HandleTable::<(), 1, 0>::budgeted(budget.clone());
    let mut child = HandleTable::<(), 1, 0>::budgeted(budget.clone());
    let first = parent
        .insert(object, Rights::READ | Rights::DUP | Rights::TRANSFER)
        .unwrap();
    let duplicate = parent
        .duplicate(first, Rights::READ | Rights::TRANSFER)
        .unwrap();
    assert_eq!(budget.count(Resource::Handles), 2);
    assert_eq!(child.reserve(1).unwrap_err(), CapabilityError::Capacity);
    assert_eq!(child.reserved_count(), 0);
    parent.close(first).unwrap();
    assert_eq!(child.reserve(2).unwrap_err(), CapabilityError::Capacity);
    assert_eq!(budget.count(Resource::Handles), 1);
    assert_eq!(child.reserved_count(), 0);
    let reservation = child.reserve(1).unwrap();
    let batch = parent
        .prepare_transfer(&[TransferSpec {
            handle: duplicate,
            rights: Rights::READ,
        }])
        .unwrap();
    let received = child.commit_transfer(reservation, batch).unwrap()[0].unwrap();
    assert_eq!(budget.count(Resource::Handles), 2);
    parent.close(duplicate).unwrap();
    assert_eq!(budget.count(Resource::Handles), 1);
    child.close(received).unwrap();
    assert_eq!(budget.count(Resource::Handles), 0);
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn reservations_own_charges_through_token_loss_and_unused_commit_refunds() {
    let budget = Budget::new(limits());
    let source = HandleTable::<(), 0, 0>::growing();
    let mut receiver = HandleTable::<(), 0, 0>::budgeted(budget.clone());
    let reservation = receiver.reserve(2).unwrap();
    assert_eq!(budget.count(Resource::Handles), 2);
    receiver
        .commit_transfer(reservation, source.prepare_transfer(&[]).unwrap())
        .unwrap();
    assert_eq!(budget.count(Resource::Handles), 0);
    {
        let _lost_reservation = receiver.reserve(2).unwrap();
    }
    assert_eq!(receiver.reserved_count(), 2);
    assert_eq!(budget.count(Resource::Handles), 2);
    drop(receiver);
    assert_eq!(budget.count(Resource::Handles), 0);
}

#[test]
fn partial_reservation_and_failed_insert_return_all_unpublished_owners() {
    let budget = Budget::new(Limits {
        handles: 1,
        ..limits()
    });
    let objects = ObjectTable::<(), 1>::new();
    let mut handles = HandleTable::<(), 1, 0>::budgeted(budget.clone());
    assert_eq!(handles.reserve(2).unwrap_err(), CapabilityError::Capacity);
    assert_eq!(budget.count(Resource::Handles), 0);
    assert_eq!(handles.reserved_count(), 0);
    let reservation = handles.reserve(1).unwrap();
    let object = objects.create(ObjectKind::Memory, ()).unwrap();
    let failed = handles.insert(object, Rights::READ).unwrap_err();
    assert_eq!(failed.error, CapabilityError::Capacity);
    assert_eq!(failed.object.reference_count(), 1);
    assert_eq!(budget.count(Resource::Handles), 1);
    drop(failed);
    handles.release_reservation(reservation).unwrap();
    assert_eq!(budget.count(Resource::Handles), 0);
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn ready_waits_remain_charged_until_consumed_or_abandoned() {
    let budget = Budget::new(limits());
    let mut tasks = TaskTable::<(), 2>::new();
    let first = tasks.spawn(()).unwrap();
    let second = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<u8, 0>::growing();
    let wait = waits.begin_budgeted(first, Some(42), &budget).unwrap();
    assert_eq!(
        waits.begin_budgeted(second, None, &budget),
        Err(WaitError::Capacity)
    );
    waits.complete(wait, 9).unwrap();
    assert_eq!(waits.take(wait), Err(WaitError::NotReady));
    waits.park(wait).unwrap();
    assert_eq!(budget.count(Resource::Waits), 1);
    assert_eq!(waits.take(wait), Ok(9));
    assert_eq!(budget.count(Resource::Waits), 0);
    let next = waits.begin_budgeted(second, None, &budget).unwrap();
    waits.abandon(next).unwrap();
    assert_eq!(budget.count(Resource::Waits), 0);
    waits.begin_budgeted(first, None, &budget).unwrap();
    drop(waits);
    assert_eq!(budget.count(Resource::Waits), 0);
}

#[test]
fn duplicate_wait_failure_refunds_its_temporary_charge() {
    let budget = Budget::new(Limits {
        waits: 2,
        ..limits()
    });
    let mut tasks = TaskTable::<(), 1>::new();
    let task = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<(), 0>::growing();
    let wait = waits.begin_budgeted(task, None, &budget).unwrap();
    assert_eq!(
        waits.begin_budgeted(task, None, &budget),
        Err(WaitError::AlreadyWaiting)
    );
    assert_eq!(budget.count(Resource::Waits), 1);
    waits.abandon(wait).unwrap();
    assert_eq!(budget.count(Resource::Waits), 0);
}

#[test]
fn simultaneous_reservations_cannot_overcommit_an_account() {
    use std::sync::Barrier;
    let budget = Budget::new(Limits {
        threads: 4,
        ..limits()
    });
    let attempted = Barrier::new(13);
    let release = Barrier::new(13);
    std::thread::scope(|scope| {
        for _ in 0..12 {
            let budget = &budget;
            let attempted = &attempted;
            let release = &release;
            scope.spawn(move || {
                let charge = budget.try_charge(Resource::Threads, 1);
                attempted.wait();
                release.wait();
                drop(charge);
            });
        }
        attempted.wait();
        assert_eq!(budget.count(Resource::Threads), 4);
        release.wait();
    });
    assert_eq!(budget.count(Resource::Threads), 0);
}
