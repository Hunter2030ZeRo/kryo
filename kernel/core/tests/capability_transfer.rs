use kryo_core::capability::{CapabilityError, Handle, HandleTable, Rights, TransferSpec};
use kryo_core::object::{ObjectKind, ObjectTable};

fn insert<'objects, const HANDLES: usize>(
    objects: &'objects ObjectTable<u64, 4>,
    handles: &mut HandleTable<'objects, u64, 4, HANDLES>,
    value: u64,
    rights: Rights,
) -> Handle {
    handles
        .insert(objects.create(ObjectKind::Memory, value).unwrap(), rights)
        .unwrap()
}

fn spec(handle: Handle, rights: Rights) -> TransferSpec {
    TransferSpec { handle, rights }
}

#[test]
fn delegation_requires_transfer_but_not_dup_and_keeps_the_original() {
    let objects = ObjectTable::<u64, 4>::new();
    let mut source = HandleTable::<u64, 4, 2>::new();
    let original = insert(
        &objects,
        &mut source,
        42,
        Rights::READ | Rights::MAP | Rights::TRANSFER,
    );
    let batch = source
        .prepare_transfer(&[spec(original, Rights::READ | Rights::MAP)])
        .unwrap();
    assert_eq!(batch.len(), 1);
    let mut receiver = HandleTable::<u64, 4, 2>::new();
    let reservation = receiver.reserve(2).unwrap();
    assert_eq!(reservation.capacity(), 2);
    let received = receiver.commit_transfer(reservation, batch).unwrap();
    let delegated = received[0].unwrap();
    assert!(received[1].is_none());
    assert_eq!(source.live_count(), 1);
    assert_eq!(receiver.reserved_count(), 0);
    assert_eq!(
        receiver
            .lookup(delegated, ObjectKind::Memory, Rights::READ | Rights::MAP)
            .unwrap()
            .with(|v| *v),
        42
    );
    assert_eq!(
        receiver
            .lookup(delegated, ObjectKind::Memory, Rights::WRITE)
            .unwrap_err(),
        CapabilityError::PermissionDenied
    );
    source.close(original).unwrap();
    assert_eq!(objects.live_count(), 1);
    receiver.close(delegated).unwrap();
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn dup_without_transfer_cannot_delegate() {
    let objects = ObjectTable::<u64, 4>::new();
    let mut source = HandleTable::<u64, 4, 1>::new();
    let original = insert(&objects, &mut source, 7, Rights::READ | Rights::DUP);
    assert_eq!(
        source
            .prepare_transfer(&[spec(original, Rights::READ)])
            .unwrap_err(),
        CapabilityError::PermissionDenied
    );
    assert_eq!(source.live_count(), 1);
}

#[test]
fn delegation_cannot_add_rights() {
    let objects = ObjectTable::<u64, 4>::new();
    let mut source = HandleTable::<u64, 4, 1>::new();
    let original = insert(&objects, &mut source, 7, Rights::READ | Rights::TRANSFER);
    assert_eq!(
        source
            .prepare_transfer(&[spec(original, Rights::READ | Rights::WRITE)])
            .unwrap_err(),
        CapabilityError::PermissionDenied
    );
    assert_eq!(
        source
            .lookup(original, ObjectKind::Memory, Rights::READ)
            .unwrap()
            .reference_count(),
        2
    );
}

#[test]
fn fixed_owner_receive_and_wait_rights_cannot_be_delegated() {
    let objects = ObjectTable::<u64, 4>::new();
    let mut source = HandleTable::<u64, 4, 2>::new();
    for (kind, owner_right) in [
        (ObjectKind::Endpoint, Rights::RECEIVE),
        (ObjectKind::Notification, Rights::WAIT),
    ] {
        let object = objects.create(kind, 1).unwrap();
        let original = source
            .insert(object, owner_right | Rights::TRANSFER)
            .unwrap();
        assert_eq!(
            source
                .prepare_transfer(&[spec(original, owner_right)])
                .unwrap_err(),
            CapabilityError::PermissionDenied
        );
    }
    assert_eq!(source.live_count(), 2);
}

#[test]
fn second_invalid_offer_rolls_back_the_first_reference() {
    let objects = ObjectTable::<u64, 4>::new();
    let mut source = HandleTable::<u64, 4, 1>::new();
    let original = insert(&objects, &mut source, 7, Rights::READ | Rights::TRANSFER);
    assert_eq!(
        source
            .prepare_transfer(&[
                spec(original, Rights::READ),
                spec(Handle::INVALID, Rights::READ)
            ])
            .unwrap_err(),
        CapabilityError::InvalidHandle
    );
    assert_eq!(
        source
            .lookup(original, ObjectKind::Memory, Rights::READ)
            .unwrap()
            .reference_count(),
        2
    );
    source.close(original).unwrap();
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn offers_are_bounded_to_two_capabilities() {
    let objects = ObjectTable::<u64, 4>::new();
    let source = HandleTable::<u64, 4, 1>::new();
    assert_eq!(
        source
            .prepare_transfer(&[spec(Handle::INVALID, Rights::READ); 3])
            .unwrap_err(),
        CapabilityError::Capacity
    );
    let mut receiver = HandleTable::<u64, 4, 3>::new();
    assert_eq!(receiver.reserve(3).unwrap_err(), CapabilityError::Capacity);
    assert_eq!(receiver.reserved_count(), 0);
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn reserved_slots_cannot_be_stolen_by_insert_duplicate_or_guessed_close() {
    let objects = ObjectTable::<u64, 4>::new();
    let mut handles = HandleTable::<u64, 4, 3>::new();
    let original = insert(&objects, &mut handles, 1, Rights::READ | Rights::DUP);
    let reservation = handles.reserve(2).unwrap();
    assert_eq!(handles.reserved_count(), 2);
    assert_eq!(
        handles.duplicate(original, Rights::READ),
        Err(CapabilityError::Capacity)
    );
    let object = objects.create(ObjectKind::Memory, 2).unwrap();
    let error = handles.insert(object, Rights::READ).unwrap_err();
    assert_eq!(error.error, CapabilityError::Capacity);
    drop(error.object);
    let guessed = Handle::from_raw((1 << 32) | 2);
    assert_eq!(handles.close(guessed), Err(CapabilityError::InvalidHandle));
    assert_eq!(handles.reserved_count(), 2);
    handles.release_reservation(reservation).unwrap();
    assert_eq!(handles.reserved_count(), 0);
    assert!(handles.duplicate(original, Rights::READ).is_ok());
}

#[test]
fn insufficient_receiver_capacity_leaves_no_partial_reservation() {
    let mut handles = HandleTable::<u64, 4, 1>::new();
    assert_eq!(handles.reserve(2).unwrap_err(), CapabilityError::Capacity);
    assert_eq!(handles.reserved_count(), 0);
    let reservation = handles.reserve(1).unwrap();
    handles.release_reservation(reservation).unwrap();
    assert_eq!(handles.reserved_count(), 0);
}

#[test]
fn foreign_reservation_error_returns_both_owners_unchanged() {
    let objects = ObjectTable::<u64, 4>::new();
    let mut source = HandleTable::<u64, 4, 1>::new();
    let original = insert(&objects, &mut source, 9, Rights::READ | Rights::TRANSFER);
    let batch = source
        .prepare_transfer(&[spec(original, Rights::READ)])
        .unwrap();
    let mut first = HandleTable::<u64, 4, 1>::new();
    let mut second = HandleTable::<u64, 4, 1>::new();
    let first_reservation = first.reserve(1).unwrap();
    let second_reservation = second.reserve(1).unwrap();
    let error = second
        .commit_transfer(first_reservation, batch)
        .unwrap_err();
    assert_eq!(error.error, CapabilityError::InvalidHandle);
    assert_eq!(first.reserved_count(), 1);
    assert_eq!(second.reserved_count(), 1);
    assert_eq!(second.live_count(), 0);
    let received = first
        .commit_transfer(error.reservation, error.batch)
        .unwrap();
    assert!(received[0].is_some());
    second.release_reservation(second_reservation).unwrap();
}

#[test]
fn foreign_release_preserves_the_reservation_for_its_original_table() {
    let mut first = HandleTable::<u64, 4, 1>::new();
    let mut second = HandleTable::<u64, 4, 1>::new();
    let reservation = first.reserve(1).unwrap();
    let own = second.reserve(1).unwrap();
    let error = second.release_reservation(reservation).unwrap_err();
    assert_eq!(error.error, CapabilityError::InvalidHandle);
    first.release_reservation(error.reservation).unwrap();
    second.release_reservation(own).unwrap();
    assert_eq!(first.reserved_count() + second.reserved_count(), 0);
}

#[test]
fn small_reservation_commit_failure_keeps_the_entire_batch() {
    let objects = ObjectTable::<u64, 4>::new();
    let mut source = HandleTable::<u64, 4, 2>::new();
    let original = insert(&objects, &mut source, 9, Rights::READ | Rights::TRANSFER);
    let batch = source
        .prepare_transfer(&[spec(original, Rights::READ); 2])
        .unwrap();
    let mut receiver = HandleTable::<u64, 4, 2>::new();
    let reservation = receiver.reserve(1).unwrap();
    let error = receiver.commit_transfer(reservation, batch).unwrap_err();
    assert_eq!(error.error, CapabilityError::Capacity);
    assert_eq!(error.batch.len(), 2);
    assert_eq!(receiver.reserved_count(), 1);
    assert_eq!(receiver.live_count(), 0);
    receiver.release_reservation(error.reservation).unwrap();
    let reservation = receiver.reserve(2).unwrap();
    let handles = receiver.commit_transfer(reservation, error.batch).unwrap();
    assert_ne!(handles[0], handles[1]);
    assert_eq!(receiver.live_count(), 2);
}

#[test]
fn staged_references_survive_source_close_before_acceptance() {
    let objects = ObjectTable::<u64, 4>::new();
    let mut source = HandleTable::<u64, 4, 1>::new();
    let original = insert(&objects, &mut source, 71, Rights::READ | Rights::TRANSFER);
    let batch = source
        .prepare_transfer(&[spec(original, Rights::READ)])
        .unwrap();
    source.close(original).unwrap();
    assert_eq!(objects.live_count(), 1);
    let mut receiver = HandleTable::<u64, 4, 1>::new();
    let reservation = receiver.reserve(1).unwrap();
    let handle = receiver.commit_transfer(reservation, batch).unwrap()[0].unwrap();
    assert_eq!(
        receiver
            .lookup(handle, ObjectKind::Memory, Rights::READ)
            .unwrap()
            .with(|v| *v),
        71
    );
    receiver.close(handle).unwrap();
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn cancelled_unaccepted_delegation_keeps_the_source_and_returns_capacity() {
    let objects = ObjectTable::<u64, 4>::new();
    let mut source = HandleTable::<u64, 4, 1>::new();
    let original = insert(&objects, &mut source, 71, Rights::READ | Rights::TRANSFER);
    let batch = source
        .prepare_transfer(&[spec(original, Rights::READ)])
        .unwrap();
    let mut receiver = HandleTable::<u64, 4, 1>::new();
    let reservation = receiver.reserve(1).unwrap();
    drop(batch);
    receiver.release_reservation(reservation).unwrap();
    assert_eq!(receiver.live_count(), 0);
    assert_eq!(receiver.reserved_count(), 0);
    assert_eq!(
        source
            .lookup(original, ObjectKind::Memory, Rights::READ)
            .unwrap()
            .with(|v| *v),
        71
    );
    source.close(original).unwrap();
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn accepted_reference_survives_sender_death_and_last_receiver_handle_close() {
    let objects = ObjectTable::<u64, 4>::new();
    let mut source = HandleTable::<u64, 4, 1>::new();
    let original = insert(&objects, &mut source, 71, Rights::READ | Rights::TRANSFER);
    let batch = source
        .prepare_transfer(&[spec(original, Rights::READ)])
        .unwrap();
    let mut receiver = HandleTable::<u64, 4, 1>::new();
    let reservation = receiver.reserve(1).unwrap();
    let handle = receiver.commit_transfer(reservation, batch).unwrap()[0].unwrap();
    drop(source);
    let retained = receiver
        .lookup(handle, ObjectKind::Memory, Rights::READ)
        .unwrap();
    receiver.close(handle).unwrap();
    assert_eq!(objects.live_count(), 1);
    assert_eq!(retained.with(|v| *v), 71);
    drop(retained);
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn receiver_table_may_move_while_a_reservation_is_pending() {
    let objects = ObjectTable::<u64, 4>::new();
    let source = HandleTable::<u64, 4, 1>::new();
    let batch = source.prepare_transfer(&[]).unwrap();
    assert!(batch.is_empty());
    let mut receiver = HandleTable::<u64, 4, 2>::new();
    let reservation = receiver.reserve(2).unwrap();
    let mut moved = receiver;
    assert_eq!(
        moved.commit_transfer(reservation, batch).unwrap(),
        [None, None]
    );
    assert_eq!(moved.reserved_count(), 0);
    let handle = insert(&objects, &mut moved, 1, Rights::READ);
    moved.close(handle).unwrap();
}

#[test]
fn zero_capability_receive_uses_no_handle_slots() {
    let mut receiver = HandleTable::<u64, 4, 0>::new();
    let source = HandleTable::<u64, 4, 0>::new();
    let batch = source.prepare_transfer(&[]).unwrap();
    let reservation = receiver.reserve(0).unwrap();
    assert_eq!(
        receiver.commit_transfer(reservation, batch).unwrap(),
        [None, None]
    );
    assert_eq!(receiver.reserved_count(), 0);
}
