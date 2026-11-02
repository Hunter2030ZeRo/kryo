use super::{
    BootFramePool, CallStats, Handles, IpcRuntime, KernelMemory, KernelObject, KernelRegion,
    Object, Objects, RefCell, Tasks,
};
use kryo_core::capability::Rights;
use kryo_core::device::{DeviceAuthority, DeviceError, DeviceState, RegisterWindow};
use kryo_core::ipc::Notification;
use kryo_core::object::ObjectKind;
use kryo_core::process::ExitStatus;
use kryo_core::process::{ExitReason, ProcessId};
use kryo_core::sched::TaskState;
use kryo_hal::{DirectMap, TimerDevice};
use kryo_interfaces::syscall::Error;

#[cfg(feature = "boot-tests")]
mod checks;

pub(crate) struct DeviceBinding {
    authority: DeviceAuthority<3>,
    hardware: Option<TimerDevice>,
}

const WINDOWS: [RegisterWindow; 3] = [
    RegisterWindow {
        offset: 0xf0,
        length: 4,
        readable: true,
        writable: false,
    },
    RegisterWindow {
        offset: 0x120,
        length: 4,
        readable: true,
        writable: true,
    },
    RegisterWindow {
        offset: 0x128,
        length: 4,
        readable: true,
        writable: true,
    },
];

pub(crate) fn access(
    object: &Object<'_>,
    owner: ProcessId,
    offset: u64,
    write: Option<u64>,
) -> Result<u64, Error> {
    let value = write
        .map(u32::try_from)
        .transpose()
        .map_err(|_| Error::InvalidArgument)?;
    let mut hardware = object.with(|object| {
        let KernelObject::Device(device) = object else {
            return Err(Error::WrongType);
        };
        let mut device = device.borrow_mut();
        device
            .authority
            .validate(owner, offset, 4, value.is_some())
            .map_err(|error| match error {
                DeviceError::PermissionDenied => Error::PermissionDenied,
                DeviceError::Revoked => Error::PeerClosed,
                _ => Error::InvalidArgument,
            })?;
        device.hardware.take().ok_or(Error::PeerClosed)
    })?;
    // The coordinator retains the strong object and owns policy exclusively.
    // No arena lock spans hardware access or an interrupt-drain window.
    let result = if let Some(value) = value {
        hardware.write32(offset, value).map(|()| 0)
    } else {
        hardware.read32(offset).map(u64::from)
    };
    object.with(|object| {
        let KernelObject::Device(device) = object else {
            unreachable!()
        };
        device.borrow_mut().hardware = Some(hardware);
    });
    result.map_err(|error| match error {
        kryo_hal::DeviceError::InvalidOffset | kryo_hal::DeviceError::InvalidValue => {
            Error::InvalidArgument
        }
        kryo_hal::DeviceError::Closed => Error::PeerClosed,
        _ => Error::OutOfResources,
    })
}

pub(crate) fn poll(ipc: &mut IpcRuntime<'_>) {
    if ipc.device.is_none() || !kryo_hal::device_irq_pending() {
        return;
    }
    if kryo_hal::take_device_irq()
        .expect("Cannot consume device IRQ")
        .is_none()
    {
        return;
    }
    if !has_wake_source(ipc) {
        return;
    }
    if let Some(signal) = &ipc.device_signal {
        signal.with(|object| {
            let KernelObject::Notification(notification) = object else {
                unreachable!()
            };
            let mut notification = notification.borrow_mut();
            if !notification.is_closed() {
                notification
                    .signal(&mut ipc.waits)
                    .expect("Device IRQ notification invariant");
            }
        });
    }
}

pub(crate) fn has_wake_source(ipc: &IpcRuntime<'_>) -> bool {
    let active = ipc.device.as_ref().is_some_and(|object| {
        object.with(|object| match object {
            KernelObject::Device(device) => {
                device.borrow().authority.state() == DeviceState::Active
            }
            _ => false,
        })
    });
    active
        && ipc.device_signal.as_ref().is_some_and(|object| {
            object.with(|object| match object {
                KernelObject::Notification(notification) => !notification.borrow().is_closed(),
                _ => false,
            })
        })
}

pub(crate) fn retire_bindings(
    tasks: &Tasks<'_, '_>,
    ipc: &mut IpcRuntime<'_>,
) -> Result<(), kryo_hal::DeviceError> {
    super::transport::retire_bindings(tasks, ipc)
        .map_err(|_| kryo_hal::DeviceError::DrainPending)?;
    let Some(object) = &ipc.device else {
        return Ok(());
    };
    let owner = object.with(|object| match object {
        KernelObject::Device(device) => device.borrow().authority.owner(),
        _ => unreachable!(),
    });
    let consumer_closed = ipc.device_signal.as_ref().is_none_or(|object| {
        object.with(|object| match object {
            KernelObject::Notification(notification) => notification.borrow().is_closed(),
            _ => true,
        })
    });
    if tasks.terminal(owner).is_none() && !consumer_closed {
        return Ok(());
    }
    let mut hardware = object.with(|object| {
        let KernelObject::Device(device) = object else {
            unreachable!()
        };
        let mut device = device.borrow_mut();
        device.authority.begin_revoke();
        device
            .hardware
            .take()
            .expect("Device hardware owner invariant")
    });
    let closed = hardware.close();
    object.with(|object| {
        let KernelObject::Device(device) = object else {
            unreachable!()
        };
        let mut device = device.borrow_mut();
        device.hardware = Some(hardware);
        if closed.is_ok() {
            device
                .authority
                .finish_revoke(|| Ok::<_, ()>(()))
                .expect("Device policy retirement invariant");
        }
    });
    closed?;
    if let Some(signal) = ipc.device_signal.take() {
        signal.with(|object| {
            let KernelObject::Notification(notification) = object else {
                unreachable!()
            };
            notification.borrow_mut().close(&mut ipc.waits);
        });
        ipc.unregister(signal.id());
    }
    ipc.device = None;
    Ok(())
}

pub(crate) fn probe(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
    required: bool,
) {
    #[cfg(feature = "boot-tests")]
    check_pending_irq();
    let hardware = match unsafe { kryo_hal::claim_timer_device() } {
        Ok(hardware) => hardware,
        Err(kryo_hal::DeviceError::UnsupportedTimer) if !required => {
            kryo_hal::emergency_write_str("DEVICE: optional HPET comparator unavailable\n");
            return;
        }
        Err(error) => panic!("Cannot claim timer device: {:?}", error),
    };
    probe_instance(pool, direct, memory, region, hardware, 0);
    let hardware = unsafe { kryo_hal::claim_timer_device() }.expect("Cannot reclaim timer device");
    probe_instance(pool, direct, memory, region, hardware, 1);
    let hardware =
        unsafe { kryo_hal::claim_timer_device() }.expect("Cannot claim replacement device");
    probe_instance(pool, direct, memory, region, hardware, 2);
    let hardware =
        unsafe { kryo_hal::claim_timer_device() }.expect("Cannot claim consumer-exit fixture");
    probe_instance(pool, direct, memory, region, hardware, 3);
    kryo_hal::emergency_write_str("DEVICE: register authority and IRQ wakeup OK cases=4\n");
}

#[inline(never)]
fn probe_instance(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
    hardware: TimerDevice,
    mode: u64,
) {
    #[cfg(feature = "boot-tests")]
    let before = pool.stats();
    let delay = (25_000_000u64 * 1_000_000).div_ceil(u64::from(hardware.period_fs().unwrap()));
    assert!(delay > 0 && delay < (1 << 31));
    let initial_irq = kryo_hal::device_irq_epoch();
    let objects = Objects::new();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::with_objects(&objects);
    let image = crate::boot::module_image("kryo-device-probe").expect("Missing device probe image");
    let mut resources = super::launch::prepare(pool, direct, memory, region, 2, image)
        .unwrap_or_else(|_| panic!("Cannot prepare device process"));
    resources.process.handles = Handles::new();
    let task = tasks
        .spawn(resources)
        .expect("Cannot publish device process");
    let owner = tasks.process_for(task).unwrap();
    let signal = objects
        .create(
            ObjectKind::Notification,
            KernelObject::Notification(RefCell::new(Notification::new(task))),
        )
        .unwrap();
    ipc.try_register(signal.try_clone().unwrap())
        .unwrap_or_else(|_| panic!("Cannot register device notification"));
    let device = objects
        .create(
            ObjectKind::Device,
            KernelObject::Device(RefCell::new(DeviceBinding {
                authority: DeviceAuthority::new(owner, WINDOWS).unwrap(),
                hardware: Some(hardware),
            })),
        )
        .unwrap();
    let resources = tasks.resources_mut(task).unwrap();
    resources.thread.context.rdi = resources
        .process
        .handles
        .insert(device.try_clone().unwrap(), Rights::READ | Rights::WRITE)
        .unwrap()
        .as_raw();
    resources.thread.context.rsi = resources
        .process
        .handles
        .insert(signal.try_clone().unwrap(), Rights::WAIT)
        .unwrap()
        .as_raw();
    resources.thread.context.rdx = delay;
    resources.thread.context.rcx = mode;
    ipc.device = Some(device.try_clone().unwrap());
    ipc.device_signal = Some(signal.try_clone().unwrap());
    #[cfg(feature = "boot-tests")]
    if mode == 1 {
        checks::expiry(&mut tasks, &mut ipc, task, &device, owner, delay as u32);
    }
    let mut calls = CallStats::default();
    let deadline = kryo_hal::monotonic_time()
        .unwrap()
        .saturating_add(2_000_000_000);
    let mut external_idle = false;
    let mut consumer_retired = false;
    for _ in 0..1024 {
        assert!(
            kryo_hal::monotonic_time().unwrap() < deadline,
            "Device process stalled"
        );
        super::expire_waits(&mut tasks, &mut ipc);
        if mode == 3 {
            let active_process = tasks.terminal(owner).is_none();
            let bound = ipc.device.is_some();
            retire_bindings(&tasks, &mut ipc).expect("Cannot retire dead IRQ consumer");
            if active_process && bound && ipc.device.is_none() {
                consumer_retired = true;
                assert_eq!(access(&device, owner, 0xf0, None), Err(Error::PeerClosed));
            }
        }
        if tasks.terminal(owner).is_some() {
            break;
        }
        if tasks.has_runnable() {
            super::smp::step(&mut tasks, &mut calls, &mut ipc, region);
            if mode == 2 && access(&device, owner, 0x120, None).is_ok_and(|config| config & 4 != 0)
            {
                tasks
                    .stop_process(
                        owner,
                        ExitStatus {
                            reason: ExitReason::Stopped,
                            code: 0,
                        },
                    )
                    .unwrap();
                ipc.settle(&mut tasks);
                break;
            }
        } else if ipc.waits.next_deadline().is_some() {
            super::idle_for_events(&mut tasks, &mut ipc);
        } else {
            external_idle = true;
            // A bootstrap watchdog bounds device failure without assigning an
            // artificial deadline to the user's external interrupt wait.
            unsafe { kryo_hal::arm_deadline(deadline) }.unwrap();
            kryo_hal::idle_event().unwrap();
            unsafe { kryo_hal::stop_timer() }.unwrap();
        }
    }
    ipc.settle(&mut tasks);
    let status = tasks.terminal(owner).expect("Device process did not exit");
    assert_eq!(
        status.reason,
        if mode == 2 {
            ExitReason::Stopped
        } else {
            ExitReason::Exited
        }
    );
    assert_eq!(
        status.code, 0,
        "Device process rejected mediated access/IRQ"
    );
    if mode < 2 {
        assert!(
            kryo_hal::device_irq_epoch() > initial_irq,
            "No actual device interrupt delivered"
        );
    }
    if mode == 0 {
        assert!(external_idle, "Device wait never reached external IRQ idle");
    }
    if mode == 3 {
        assert!(
            consumer_retired,
            "IRQ consumer exit did not revoke the live process's device"
        );
    }
    #[cfg(feature = "boot-tests")]
    if mode == 1 {
        let retained = pool.stats();
        unsafe { kryo_hal::fail_next_device_close() }.unwrap();
        assert_eq!(
            retire_bindings(&tasks, &mut ipc),
            Err(kryo_hal::DeviceError::DrainPending)
        );
        assert_eq!(access(&device, owner, 0xf0, None), Err(Error::PeerClosed));
        assert!(ipc.device.is_some() && ipc.device_signal.is_some());
        assert_eq!(tasks.state(task), Ok(TaskState::Dying));
        assert_eq!(pool.stats(), retained);
        assert!(matches!(
            unsafe { kryo_hal::claim_timer_device() },
            Err(kryo_hal::DeviceError::Busy)
        ));
    }
    retire_bindings(&tasks, &mut ipc).expect("Cannot retire device hardware; resources retained");
    #[cfg(feature = "boot-tests")]
    if mode == 1 {
        kryo_hal::emergency_write_str("DEVICE: retirement retry retains owners\n");
    }
    assert_eq!(access(&device, owner, 0xf0, None), Err(Error::PeerClosed));
    assert_eq!(tasks.state(task), Ok(TaskState::Dying));
    let mut dying = [None; 4];
    for (slot, id) in dying.iter_mut().zip(tasks.ids()) {
        *slot = Some(id);
    }
    for id in dying.into_iter().flatten() {
        assert_eq!(tasks.state(id), Ok(TaskState::Dying));
        drop(
            tasks
                .reap(id, |resources| resources.close(pool, region))
                .unwrap(),
        );
    }
    drop(ipc);
    drop(device);
    drop(signal);
    assert_eq!(objects.live_count(), 0);
    #[cfg(feature = "boot-tests")]
    assert_eq!(pool.stats(), before);
}

#[cfg(feature = "boot-tests")]
fn check_pending_irq() {
    let mut hardware =
        unsafe { kryo_hal::claim_timer_device() }.expect("Cannot claim pending-IRQ fixture");
    let delay = (1_000_000u64 * 1_000_000).div_ceil(u64::from(hardware.period_fs().unwrap()));
    let now = hardware.read32(0xf0).unwrap();
    hardware
        .write32(0x128, now.wrapping_add(delay as u32))
        .unwrap();
    hardware.write32(0x120, 0x4104).unwrap();
    let epoch = kryo_hal::device_irq_epoch();
    let deadline = kryo_hal::monotonic_time()
        .unwrap()
        .saturating_add(100_000_000);
    while !kryo_hal::device_irq_queued().unwrap() {
        assert!(
            kryo_hal::monotonic_time().unwrap() < deadline,
            "Device IRQ did not queue"
        );
        core::hint::spin_loop();
    }
    assert_eq!(kryo_hal::device_irq_epoch(), epoch);
    hardware.close().expect("Cannot drain queued device IRQ");
    assert!(kryo_hal::device_irq_epoch() > epoch);
    assert!(!kryo_hal::device_irq_queued().unwrap());
    kryo_hal::emergency_write_str("DEVICE: queued IRQ retired before reuse\n");
}
