use super::{IpcRuntime, KernelObject, Object, Tasks};
use core::cell::RefCell;
use kryo_core::ipc::Notification;
use kryo_core::object::ObjectKind;
use kryo_core::process::ProcessId;
use kryo_core::sched::{ReturnReason, TaskState};
use kryo_core::task::TaskId;
use kryo_interfaces::syscall::Error;

pub(super) fn expiry<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    task: TaskId,
    device: &Object<'objects>,
    owner: ProcessId,
    delay: u32,
) {
    registration_capacity(ipc, task);
    let context = tasks.thread_mut(task).unwrap().context;
    let signal = ipc.device_signal.as_ref().unwrap().try_clone().unwrap();
    let expires = kryo_hal::monotonic_time()
        .unwrap()
        .checked_add(1_000_000)
        .unwrap();
    park(tasks, ipc, task, &signal, Some(expires));
    let ticks = delay / 8;
    assert!(ticks > 0);
    super::access(device, owner, 0x120, Some(0x4100)).unwrap();
    let counter = super::access(device, owner, 0xf0, None).unwrap() as u32;
    super::access(
        device,
        owner,
        0x128,
        Some(u64::from(counter.wrapping_add(ticks))),
    )
    .unwrap();
    super::access(device, owner, 0x120, Some(0x4104)).unwrap();

    let bound = kryo_hal::monotonic_time()
        .unwrap()
        .checked_add(100_000_000)
        .unwrap();
    while !kryo_hal::device_irq_queued().unwrap() {
        assert!(
            kryo_hal::monotonic_time().unwrap() < bound,
            "Comparator did not queue its IRQ"
        );
        core::hint::spin_loop();
    }
    assert!(kryo_hal::monotonic_time().unwrap() >= expires);
    unsafe { kryo_hal::arm_deadline(bound) }.unwrap();
    while !kryo_hal::device_irq_pending() {
        assert!(
            kryo_hal::monotonic_time().unwrap() < bound,
            "Queued device IRQ did not reach its handler"
        );
        kryo_hal::idle_event().unwrap();
    }
    unsafe { kryo_hal::stop_timer() }.unwrap();
    crate::runtime::expire_waits(tasks, ipc);
    assert_eq!(tasks.state(task), Ok(TaskState::Runnable));
    let thread = tasks.thread_mut(task).unwrap();
    assert!(thread.wait.is_none());
    assert_eq!(thread.context.rax, Error::TimedOut.status());
    assert!(!kryo_hal::device_irq_pending());
    assert!(ipc.waits.next_deadline().is_none());

    // The late event survives expiry and completes a fresh wait without a
    // second comparator event. Only the first wait receives the timeout.
    let epoch = kryo_hal::device_irq_epoch();
    super::access(device, owner, 0x120, Some(0x4100)).unwrap();
    park(tasks, ipc, task, &signal, None);
    ipc.settle(tasks);
    assert_eq!(tasks.state(task), Ok(TaskState::Runnable));
    let thread = tasks.thread_mut(task).unwrap();
    assert!(thread.wait.is_none());
    assert_eq!(thread.context.rax, 0);
    assert_eq!(kryo_hal::device_irq_epoch(), epoch);
    assert!(ipc.waits.next_ready().is_none());
    thread.context = context;
    kryo_hal::emergency_write_str("DEVICE: expired wait precedes queued IRQ\n");
}

fn park<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    task: TaskId,
    signal: &Object<'objects>,
    deadline: Option<u64>,
) {
    assert_eq!(tasks.dispatch().unwrap(), Some(task));
    assert_eq!(
        tasks.returned(task, ReturnReason::Call),
        Ok(TaskState::Blocked)
    );
    let wait = if let Some(deadline) = deadline {
        ipc.waits.begin_until(task, deadline)
    } else {
        ipc.waits.begin(task)
    }
    .unwrap();
    signal.with(|object| {
        let KernelObject::Notification(notification) = object else {
            unreachable!()
        };
        notification
            .borrow_mut()
            .wait(task, wait, &mut ipc.waits)
            .unwrap();
    });
    tasks.thread_mut(task).unwrap().wait = Some(wait);
    ipc.waits.park(wait).unwrap();
}

fn registration_capacity<'objects>(ipc: &mut IpcRuntime<'objects>, owner: TaskId) {
    let arena = ipc.arena.unwrap();
    let before = arena.live_count();
    let mut retained = [const { None }; 3];
    for slot in &mut retained {
        let object = arena
            .create(
                ObjectKind::Notification,
                KernelObject::Notification(RefCell::new(Notification::new(owner))),
            )
            .unwrap();
        ipc.try_register(object.try_clone().unwrap()).unwrap();
        *slot = Some(object);
    }
    let extra = arena
        .create(
            ObjectKind::Notification,
            KernelObject::Notification(RefCell::new(Notification::new(owner))),
        )
        .unwrap();
    let id = extra.id();
    let returned = ipc.try_register(extra).unwrap_err();
    assert_eq!(returned.id(), id);
    assert_eq!(returned.reference_count(), 1);
    drop(returned);
    for object in retained.into_iter().flatten() {
        ipc.unregister(object.id());
    }
    assert_eq!(arena.live_count(), before);
}
