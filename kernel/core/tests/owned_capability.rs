use kryo_core::capability::{CapabilityError, HandleTable, Rights, TransferBatch};
use kryo_core::object::{ObjectKind, ObjectTable};

#[test]
fn owned_reference_commits_to_reserved_capacity_without_cloning() {
    let objects = ObjectTable::<u64, 2>::new();
    let object = objects.create(ObjectKind::ProcessControl, 41).unwrap();
    let retained = object.try_clone().unwrap();
    let rights = Rights::INSPECT | Rights::WAIT | Rights::CONTROL | Rights::DUP | Rights::TRANSFER;
    let mut handles = HandleTable::<u64, 2, 1>::new();
    let reservation = handles.reserve(1).unwrap();
    let batch = TransferBatch::from_object(object, rights);
    assert_eq!(batch.len(), 1);
    assert_eq!(retained.reference_count(), 2);
    let unrelated = objects.create(ObjectKind::Memory, 42).unwrap();
    let rejected = handles.insert(unrelated, Rights::READ).unwrap_err();
    assert_eq!(rejected.error, CapabilityError::Capacity);
    drop(rejected.object);

    let [Some(handle), None] = handles.commit_transfer(reservation, batch).unwrap() else {
        panic!("one owned reference expected");
    };
    assert_eq!(handles.reserved_count(), 0);
    assert_eq!(handles.live_count(), 1);
    assert_eq!(retained.reference_count(), 2);
    let granted = handles
        .lookup(handle, ObjectKind::ProcessControl, rights)
        .unwrap();
    assert_eq!(granted.id(), retained.id());
    assert_eq!(granted.with(|value| *value), 41);
    drop(granted);
    assert_eq!(
        handles
            .lookup(handle, ObjectKind::ProcessControl, Rights::EXECUTE)
            .unwrap_err(),
        CapabilityError::PermissionDenied
    );
    handles.close(handle).unwrap();
    assert_eq!(retained.reference_count(), 1);
    drop(retained);
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn rejected_owned_commit_returns_the_same_reference_for_retry() {
    let objects = ObjectTable::<u64, 1>::new();
    let object = objects.create(ObjectKind::ProcessControl, 7).unwrap();
    let id = object.id();
    let mut handles = HandleTable::<u64, 1, 1>::new();
    let too_small = handles.reserve(0).unwrap();
    let error = handles
        .commit_transfer(
            too_small,
            TransferBatch::from_object(object, Rights::INSPECT),
        )
        .unwrap_err();
    assert_eq!(error.error, CapabilityError::Capacity);
    assert_eq!(error.batch.len(), 1);
    assert_eq!(objects.live_count(), 1);
    handles.release_reservation(error.reservation).unwrap();
    let reserved = handles.reserve(1).unwrap();
    let [Some(handle), None] = handles.commit_transfer(reserved, error.batch).unwrap() else {
        panic!("owned reference retry expected");
    };
    let installed = handles
        .lookup(handle, ObjectKind::ProcessControl, Rights::INSPECT)
        .unwrap();
    assert_eq!(installed.id(), id);
    assert_eq!(installed.reference_count(), 2);
    drop(installed);
    handles.close(handle).unwrap();
    assert_eq!(objects.live_count(), 0);
}
