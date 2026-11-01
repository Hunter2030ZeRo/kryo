use core::cell::{Cell, RefCell};
use kryo_core::capability::{CapabilityError, HandleTable, Rights};
use kryo_core::device::{DeviceAuthority, DeviceError, DeviceState, RegisterWindow, RevokeError};
use kryo_core::object::{ObjectKind, ObjectTable};
use kryo_core::process::{ProcessId, ProcessTable};

const COUNTER: RegisterWindow = RegisterWindow {
    offset: 0xf0,
    length: 8,
    readable: true,
    writable: false,
};
const CONFIG: RegisterWindow = RegisterWindow {
    offset: 0x120,
    length: 8,
    readable: true,
    writable: true,
};
const COMPARATOR: RegisterWindow = RegisterWindow {
    offset: 0x128,
    length: 8,
    readable: true,
    writable: true,
};

#[test]
fn register_windows_allow_only_the_declared_aligned_words() {
    let owner = owner();
    let device = DeviceAuthority::new(owner, [COUNTER, CONFIG, COMPARATOR]).unwrap();
    assert_eq!(device.owner(), owner);
    assert_eq!(device.state(), DeviceState::Active);
    for offset in [0xf0, 0xf4, 0x120, 0x124, 0x128, 0x12c] {
        assert_eq!(device.validate(owner, offset, 4, false), Ok(()));
    }
    for offset in [0x120, 0x124, 0x128, 0x12c] {
        assert_eq!(device.validate(owner, offset, 4, true), Ok(()));
    }
    for offset in [0xf0, 0xf4] {
        assert_eq!(
            device.validate(owner, offset, 4, true),
            Err(DeviceError::PermissionDenied)
        );
    }
    for offset in [0, 0xec, 0xf8, 0x100, 0x11c, 0x130, u64::MAX - 3] {
        assert_eq!(
            device.validate(owner, offset, 4, false),
            Err(DeviceError::InvalidAccess)
        );
    }
}

#[test]
fn access_size_alignment_and_full_width_are_checked_before_access() {
    let owner = owner();
    let device = DeviceAuthority::new(owner, [COUNTER]).unwrap();
    for size in [0, 1, 2, 8, 16, u64::MAX] {
        assert_eq!(
            device.validate(owner, 0xf4, size, false),
            Err(DeviceError::InvalidAccess)
        );
    }
    for offset in [0xef, 0xf1, 0xf5, 0xf6] {
        assert_eq!(
            device.validate(owner, offset, 4, false),
            Err(DeviceError::InvalidAccess)
        );
    }
    assert_eq!(device.validate(owner, 0xf4, 4, false), Ok(()));
}

#[test]
fn construction_rejects_empty_misaligned_overflowing_and_overlapping_windows() {
    let owner = owner();
    assert!(matches!(
        DeviceAuthority::new(owner, []),
        Err(DeviceError::InvalidWindow)
    ));
    for window in [
        RegisterWindow {
            length: 0,
            ..COUNTER
        },
        RegisterWindow {
            length: 3,
            ..COUNTER
        },
        RegisterWindow {
            offset: 1,
            ..COUNTER
        },
        RegisterWindow {
            offset: u64::MAX - 3,
            ..COUNTER
        },
        RegisterWindow {
            readable: false,
            ..COUNTER
        },
    ] {
        assert!(matches!(
            DeviceAuthority::new(owner, [window]),
            Err(DeviceError::InvalidWindow)
        ));
    }
    for windows in [
        [COUNTER, COUNTER],
        [
            COUNTER,
            RegisterWindow {
                offset: 0xf4,
                ..COUNTER
            },
        ],
        [
            RegisterWindow {
                offset: 0xec,
                ..COUNTER
            },
            COUNTER,
        ],
    ] {
        assert!(matches!(
            DeviceAuthority::new(owner, windows),
            Err(DeviceError::OverlappingWindows)
        ));
    }
    assert!(DeviceAuthority::new(owner, [COMPARATOR, COUNTER, CONFIG]).is_ok());
}

#[test]
fn write_only_windows_do_not_grant_reads() {
    let owner = owner();
    let device = DeviceAuthority::new(
        owner,
        [RegisterWindow {
            readable: false,
            ..CONFIG
        }],
    )
    .unwrap();
    assert_eq!(device.validate(owner, 0x120, 4, true), Ok(()));
    assert_eq!(
        device.validate(owner, 0x120, 4, false),
        Err(DeviceError::PermissionDenied)
    );
}

#[test]
fn process_slot_reuse_does_not_inherit_device_authority() {
    let mut processes = ProcessTable::<(), 1, 1>::new();
    let first = processes.insert(()).unwrap();
    let device = DeviceAuthority::new(first, [CONFIG]).unwrap();
    processes.abort(first, |_| Ok::<_, ()>(())).unwrap();
    let second = processes.insert(()).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        device.validate(second, 0x120, 4, true),
        Err(DeviceError::PermissionDenied)
    );
    // Process liveness is image policy; this value retains the original identity.
    assert_eq!(device.owner(), first);
}

#[test]
fn revocation_blocks_access_before_cleanup_and_failed_cleanup_is_retryable() {
    let owner = owner();
    let mut device = DeviceAuthority::new(owner, [CONFIG]).unwrap();
    let attempts = Cell::new(0);
    assert_eq!(
        device.finish_revoke(|| {
            attempts.set(attempts.get() + 1);
            Ok::<_, ()>(())
        }),
        Err(RevokeError::State(DeviceError::NotQuiescing))
    );
    assert_eq!(attempts.get(), 0);
    assert!(device.begin_revoke());
    assert!(!device.begin_revoke());
    assert_eq!(device.state(), DeviceState::Quiescing);
    assert_eq!(
        device.validate(owner, 0x120, 4, true),
        Err(DeviceError::Revoked)
    );
    assert_eq!(
        device.finish_revoke(|| {
            attempts.set(attempts.get() + 1);
            Err("IRQ still pending")
        }),
        Err(RevokeError::Cleanup("IRQ still pending"))
    );
    assert_eq!(device.state(), DeviceState::Quiescing);
    assert_eq!(
        device.validate(owner, 0x120, 4, false),
        Err(DeviceError::Revoked)
    );
    device
        .finish_revoke(|| {
            attempts.set(attempts.get() + 1);
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(device.state(), DeviceState::Retired);
    assert!(!device.begin_revoke());
    assert_eq!(
        device.validate(owner, 0x120, 4, false),
        Err(DeviceError::Revoked)
    );
    device
        .finish_revoke(|| {
            attempts.set(attempts.get() + 1);
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(attempts.get(), 2);
}

#[test]
fn capability_attenuation_and_register_permissions_are_both_required() {
    let owner = owner();
    let objects = ObjectTable::<RefCell<DeviceAuthority<2>>, 1>::new();
    let mut handles = HandleTable::<_, 1, 3>::new();
    let device = objects
        .create(
            ObjectKind::Device,
            RefCell::new(DeviceAuthority::new(owner, [COUNTER, CONFIG]).unwrap()),
        )
        .unwrap();
    let handle = handles
        .insert(device, Rights::READ | Rights::WRITE | Rights::DUP)
        .unwrap();
    let readonly = handles.duplicate(handle, Rights::READ).unwrap();
    assert_eq!(
        handles
            .lookup(readonly, ObjectKind::Device, Rights::WRITE)
            .unwrap_err(),
        CapabilityError::PermissionDenied
    );
    assert_eq!(
        handles
            .lookup(handle, ObjectKind::Device, Rights::MAP)
            .unwrap_err(),
        CapabilityError::PermissionDenied
    );
    let writable = handles
        .lookup(handle, ObjectKind::Device, Rights::WRITE)
        .unwrap();
    assert_eq!(
        writable.with(|device| device.borrow().validate(owner, 0xf0, 4, true)),
        Err(DeviceError::PermissionDenied)
    );
    assert_eq!(
        writable.with(|device| device.borrow().validate(owner, 0x120, 4, true)),
        Ok(())
    );
    let readonly = handles
        .lookup(readonly, ObjectKind::Device, Rights::READ)
        .unwrap();
    assert_eq!(
        readonly.with(|device| device.borrow().validate(owner, 0xf0, 4, false)),
        Ok(())
    );
}

#[test]
fn closing_a_handle_does_not_revoke_retained_device_state_or_alias_a_replacement() {
    let owner = owner();
    let objects = ObjectTable::<RefCell<DeviceAuthority<1>>, 1>::new();
    let mut handles = HandleTable::<_, 1, 1>::new();
    let object = objects
        .create(
            ObjectKind::Device,
            RefCell::new(DeviceAuthority::new(owner, [CONFIG]).unwrap()),
        )
        .unwrap();
    let old_id = object.id();
    let retained = object.try_clone().unwrap();
    let old_handle = handles.insert(object, Rights::READ).unwrap();
    handles.close(old_handle).unwrap();
    assert_eq!(objects.live_count(), 1);
    assert_eq!(
        retained.with(|device| device.borrow().validate(owner, 0x120, 4, false)),
        Ok(())
    );
    retained.with(|device| {
        let mut device = device.borrow_mut();
        device.begin_revoke();
        device.finish_revoke(|| Ok::<_, ()>(())).unwrap();
    });
    drop(retained);
    let replacement = objects
        .create(
            ObjectKind::Device,
            RefCell::new(DeviceAuthority::new(owner, [CONFIG]).unwrap()),
        )
        .unwrap();
    assert_ne!(replacement.id(), old_id);
    let new_handle = handles.insert(replacement, Rights::READ).unwrap();
    assert_ne!(new_handle, old_handle);
    assert_eq!(
        handles
            .lookup(old_handle, ObjectKind::Device, Rights::READ)
            .unwrap_err(),
        CapabilityError::InvalidHandle
    );
}

fn owner() -> ProcessId {
    ProcessTable::<(), 1, 1>::new().insert(()).unwrap()
}
