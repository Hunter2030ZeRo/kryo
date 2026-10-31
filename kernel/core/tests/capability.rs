use core::cell::Cell;
use kryo_core::capability::{CapabilityError, Handle, HandleTable, Rights};
use kryo_core::object::{ObjectError, ObjectKind, ObjectTable};
use std::rc::Rc;

#[test]
fn handles_reject_zero_unknown_slots_and_stale_generations() {
    let objects = ObjectTable::<u64, 2>::new();
    let mut handles = HandleTable::<u64, 2, 2>::new();
    for raw in [0, 1, 1 << 32, u64::MAX] {
        assert_eq!(
            handles
                .lookup(
                    Handle::from_raw(raw),
                    ObjectKind::TaskControl,
                    Rights::INSPECT
                )
                .unwrap_err(),
            CapabilityError::InvalidHandle
        );
    }
    let object = objects.create(ObjectKind::TaskControl, 42).unwrap();
    let old = handles.insert(object, Rights::INSPECT).unwrap();
    assert_ne!(old.as_raw(), 0);
    handles.close(old).unwrap();
    assert_eq!(handles.close(old), Err(CapabilityError::InvalidHandle));
    let object = objects.create(ObjectKind::TaskControl, 99).unwrap();
    let new = handles.insert(object, Rights::INSPECT).unwrap();
    assert_ne!(old, new);
    assert_eq!(
        handles
            .lookup(old, ObjectKind::TaskControl, Rights::INSPECT)
            .unwrap_err(),
        CapabilityError::InvalidHandle
    );
    assert_eq!(
        handles
            .lookup(new, ObjectKind::TaskControl, Rights::INSPECT)
            .unwrap()
            .with(|v| *v),
        99
    );
}

#[test]
fn lookup_checks_kind_and_required_rights_before_retaining_reference() {
    let objects = ObjectTable::<u64, 1>::new();
    let object = objects.create(ObjectKind::TaskControl, 42).unwrap();
    let monitor = object.try_clone().unwrap();
    let mut handles = HandleTable::<u64, 1, 1>::new();
    let handle = handles.insert(object, Rights::INSPECT).unwrap();
    let count = monitor.reference_count();
    assert_eq!(
        handles
            .lookup(handle, ObjectKind::Memory, Rights::INSPECT)
            .unwrap_err(),
        CapabilityError::WrongKind
    );
    assert_eq!(
        handles
            .lookup(handle, ObjectKind::TaskControl, Rights::CONTROL)
            .unwrap_err(),
        CapabilityError::PermissionDenied
    );
    assert_eq!(monitor.reference_count(), count);
}

#[test]
fn duplicate_requires_authority_and_cannot_amplify_rights() {
    let objects = ObjectTable::<u64, 1>::new();
    let mut handles = HandleTable::<u64, 1, 4>::new();
    let object = objects.create(ObjectKind::TaskControl, 42).unwrap();
    let handle = handles
        .insert(object, Rights::INSPECT | Rights::DUP)
        .unwrap();
    assert_eq!(
        handles.duplicate(handle, Rights::CONTROL),
        Err(CapabilityError::PermissionDenied)
    );
    let limited = handles.duplicate(handle, Rights::INSPECT).unwrap();
    assert_eq!(
        handles
            .lookup(limited, ObjectKind::TaskControl, Rights::CONTROL)
            .unwrap_err(),
        CapabilityError::PermissionDenied
    );
    assert_eq!(
        handles.duplicate(limited, Rights::INSPECT),
        Err(CapabilityError::PermissionDenied)
    );
    assert_eq!(
        handles
            .lookup(limited, ObjectKind::TaskControl, Rights::INSPECT)
            .unwrap()
            .with(|v| *v),
        42
    );
    assert_eq!(handles.live_count(), 2);
}

#[test]
fn failed_duplicate_does_not_leak_an_object_reference() {
    let objects = ObjectTable::<u64, 1>::new();
    let object = objects.create(ObjectKind::TaskControl, 42).unwrap();
    let monitor = object.try_clone().unwrap();
    let mut handles = HandleTable::<u64, 1, 1>::new();
    let handle = handles
        .insert(object, Rights::INSPECT | Rights::DUP)
        .unwrap();
    let count = monitor.reference_count();
    assert_eq!(
        handles.duplicate(handle, Rights::INSPECT),
        Err(CapabilityError::Capacity)
    );
    assert_eq!(monitor.reference_count(), count);
}

#[test]
fn close_removes_one_handle_but_does_not_revoke_other_references() {
    let objects = ObjectTable::<u64, 1>::new();
    let mut first = HandleTable::<u64, 1, 2>::new();
    let mut second = HandleTable::<u64, 1, 1>::new();
    let object = objects.create(ObjectKind::TaskControl, 42).unwrap();
    let delegated = object.try_clone().unwrap();
    let original = first.insert(object, Rights::INSPECT | Rights::DUP).unwrap();
    let duplicate = first.duplicate(original, Rights::INSPECT).unwrap();
    let peer = second.insert(delegated, Rights::INSPECT).unwrap();
    let retained = first
        .lookup(original, ObjectKind::TaskControl, Rights::INSPECT)
        .unwrap();
    first.close(original).unwrap();
    first.close(duplicate).unwrap();
    second.close(peer).unwrap();
    assert_eq!(first.live_count(), 0);
    assert_eq!(objects.live_count(), 1);
    assert_eq!(retained.with(|value| *value), 42);
    drop(retained);
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn insertion_failure_returns_the_original_strong_reference() {
    let objects = ObjectTable::<u64, 1>::new();
    let object = objects.create(ObjectKind::TaskControl, 42).unwrap();
    let id = object.id();
    let mut handles = HandleTable::<u64, 1, 0>::new();
    let error = handles.insert(object, Rights::INSPECT).unwrap_err();
    assert_eq!(error.error, CapabilityError::Capacity);
    assert_eq!(error.object.id(), id);
    assert_eq!(error.object.reference_count(), 1);
    drop(error.object);
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn live_references_prevent_arena_slot_reuse() {
    let objects = ObjectTable::<u64, 1>::new();
    let object = objects.create(ObjectKind::TaskControl, 42).unwrap();
    let old_id = object.id();
    let retained = object.try_clone().unwrap();
    drop(object);
    let failure = objects.create(ObjectKind::TaskControl, 99).unwrap_err();
    assert_eq!(failure.error, ObjectError::Capacity);
    assert_eq!(failure.value, 99);
    assert_eq!(retained.with(|value| *value), 42);
    drop(retained);
    let new = objects.create(ObjectKind::TaskControl, 99).unwrap();
    assert_ne!(old_id, new.id());
}

struct Resource(Rc<Cell<usize>>);

impl Drop for Resource {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn payload_is_destroyed_only_after_the_last_strong_reference() {
    let drops = Rc::new(Cell::new(0));
    let objects = ObjectTable::<Resource, 1>::new();
    let object = objects
        .create(ObjectKind::TaskControl, Resource(drops.clone()))
        .unwrap();
    let retained = object.try_clone().unwrap();
    drop(object);
    assert_eq!(drops.get(), 0);
    drop(retained);
    assert_eq!(drops.get(), 1);
}

#[test]
fn rights_reject_unknown_bits() {
    assert_eq!(Rights::from_bits(128), None);
    assert_eq!(Rights::from_bits(u64::MAX), None);
    assert_eq!(Rights::from_bits(0).unwrap().bits(), 0);
    assert_eq!(
        Rights::from_bits(120).unwrap(),
        Rights::SEND | Rights::RECEIVE | Rights::WAIT | Rights::SIGNAL
    );
    assert_eq!(
        Rights::from_bits(7).unwrap(),
        Rights::INSPECT | Rights::CONTROL | Rights::DUP
    );
}

#[test]
fn dropping_a_handle_table_releases_its_strong_references() {
    let objects = ObjectTable::<u64, 1>::new();
    let mut handles = HandleTable::<u64, 1, 1>::new();
    let object = objects.create(ObjectKind::TaskControl, 42).unwrap();
    handles.insert(object, Rights::INSPECT).unwrap();
    assert_eq!(objects.live_count(), 1);
    drop(handles);
    assert_eq!(objects.live_count(), 0);
}
