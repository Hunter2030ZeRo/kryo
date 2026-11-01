use super::time_programs as code;
use super::{
    BootFramePool, CallStats, DirectMap, IpcRuntime, KernelMemory, KernelObject, KernelRegion,
    Objects, RefCell, Resources, Rights, TaskState, Tasks, idle_until_deadline, step,
};
use kryo_core::ipc::{Endpoint, Notification};
use kryo_core::object::ObjectKind;
use kryo_hal::UserReason;

#[derive(Clone, Copy, Debug)]
enum Case {
    Sleep,
    Queued,
    Accepted,
    NotificationTimeout,
    NotificationEvent,
    CancelSleep,
    Expired,
    ReceiveRetry,
    CapabilityTimeout,
    AcceptedCapability,
}

pub(super) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let mut idle_returns = 0;
    for case in [
        Case::Sleep,
        Case::Queued,
        Case::Accepted,
        Case::NotificationTimeout,
        Case::NotificationEvent,
        Case::CancelSleep,
        Case::Expired,
        Case::ReceiveRetry,
        Case::CapabilityTimeout,
        Case::AcceptedCapability,
    ] {
        idle_returns += run_case(case, pool, direct, kernel, region);
    }
    idle_returns += check_early_interrupt(pool, direct, kernel, region);
    assert!(idle_returns >= 7);
    let _ = kryo_hal::print(format_args!(
        "TIME: deadlines and wakeup OK cases=11 idle={}\n",
        idle_returns
    ));
}

fn run_case(
    case: Case,
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) -> usize {
    let before = pool.stats();
    let objects = Objects::new();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::with_objects(&objects);
    let mut calls = CallStats::default();
    let (first_code, second_code, expected) = match case {
        Case::Sleep => (code::sleep_checks(), code::exit(60), [61, 60]),
        Case::Queued => (code::timed_client(false), code::waiting_owner(63), [62, 63]),
        Case::Accepted => (code::late_reply(), code::timed_client(true), [64, 65]),
        Case::NotificationTimeout => (code::notification_timeout(), code::exit(60), [66, 60]),
        Case::NotificationEvent => (code::notification_event(), code::signaller(), [67, 68]),
        Case::CancelSleep => (code::cancelled_sleep(), code::canceller(), [69, 70]),
        Case::Expired => (code::expired_wait(), code::exit(60), [71, 60]),
        Case::ReceiveRetry => (code::receive_retry(), code::notified_sender(), [72, 73]),
        Case::CapabilityTimeout => (code::timed_capability(), code::waiting_owner(75), [74, 75]),
        Case::AcceptedCapability => (
            code::accepted_capability_receiver(),
            code::accepted_capability_sender(),
            [77, 76],
        ),
    };
    let first = tasks
        .spawn(Resources::new(pool, direct, kernel, region, 2, &first_code))
        .unwrap();
    let second = tasks
        .spawn(Resources::new(
            pool,
            direct,
            kernel,
            region,
            7,
            &second_code,
        ))
        .unwrap();

    match case {
        Case::Queued
        | Case::Accepted
        | Case::ReceiveRetry
        | Case::CapabilityTimeout
        | Case::AcceptedCapability => {
            let owner = if matches!(
                case,
                Case::Accepted | Case::ReceiveRetry | Case::AcceptedCapability
            ) {
                first
            } else {
                second
            };
            let sender = if owner == first { second } else { first };
            let object = objects
                .create(
                    ObjectKind::Endpoint,
                    KernelObject::Endpoint(RefCell::new(Endpoint::new(owner))),
                )
                .unwrap();
            let mut receiver = tasks.resources_mut(owner).unwrap();
            receiver.context.rbp = receiver
                .process
                .handles
                .insert(object.try_clone().unwrap(), Rights::RECEIVE)
                .unwrap()
                .as_raw();
            let mut client = tasks.resources_mut(sender).unwrap();
            client.context.rbp = client
                .process
                .handles
                .insert(object.try_clone().unwrap(), Rights::SEND)
                .unwrap()
                .as_raw();
            ipc.register(object);
            if matches!(case, Case::Accepted) {
                let control = objects
                    .create(ObjectKind::TaskControl, KernelObject::TaskControl(owner))
                    .unwrap();
                let mut client = tasks.resources_mut(sender).unwrap();
                client.context.r15 = client
                    .process
                    .handles
                    .insert(control, Rights::INSPECT)
                    .unwrap()
                    .as_raw();
            }
            {
                let waiter = match case {
                    Case::Accepted | Case::AcceptedCapability => first,
                    Case::Queued | Case::CapabilityTimeout | Case::ReceiveRetry => second,
                    _ => unreachable!(),
                };
                let signaller = if waiter == first { second } else { first };
                let notification = objects
                    .create(
                        ObjectKind::Notification,
                        KernelObject::Notification(RefCell::new(Notification::new(waiter))),
                    )
                    .unwrap();
                let mut receiver = tasks.resources_mut(waiter).unwrap();
                receiver.context.rbx = receiver
                    .process
                    .handles
                    .insert(notification.try_clone().unwrap(), Rights::WAIT)
                    .unwrap()
                    .as_raw();
                let mut sender = tasks.resources_mut(signaller).unwrap();
                sender.context.rbx = sender
                    .process
                    .handles
                    .insert(notification.try_clone().unwrap(), Rights::SIGNAL)
                    .unwrap()
                    .as_raw();
                ipc.register(notification);
            }
        }
        Case::NotificationTimeout | Case::NotificationEvent | Case::Expired => {
            let object = objects
                .create(
                    ObjectKind::Notification,
                    KernelObject::Notification(RefCell::new(Notification::new(first))),
                )
                .unwrap();
            let mut consumer = tasks.resources_mut(first).unwrap();
            consumer.context.rbp = consumer
                .process
                .handles
                .insert(object.try_clone().unwrap(), Rights::WAIT | Rights::SIGNAL)
                .unwrap()
                .as_raw();
            if matches!(case, Case::NotificationEvent) {
                let mut producer = tasks.resources_mut(second).unwrap();
                producer.context.rbp = producer
                    .process
                    .handles
                    .insert(object.try_clone().unwrap(), Rights::SIGNAL)
                    .unwrap()
                    .as_raw();
            }
            ipc.register(object);
        }
        Case::CancelSleep => {
            let control = objects
                .create(ObjectKind::TaskControl, KernelObject::TaskControl(first))
                .unwrap();
            let mut canceller = tasks.resources_mut(second).unwrap();
            let handle = canceller
                .process
                .handles
                .insert(control, Rights::CONTROL | Rights::INSPECT)
                .unwrap()
                .as_raw();
            canceller.context.rbp = handle;
            canceller.context.r15 = handle;
        }
        Case::Sleep => {}
    }

    let mut reaped = [false; 2];
    let mut idle_returns = 0;
    let mut queued_wait_observed = false;
    for _ in 0..256 {
        if tasks.live_count() == 0 {
            break;
        }
        if tasks.has_runnable() {
            let (_, outcome) = step(&mut tasks, &mut calls, &mut ipc);
            assert_ne!(
                outcome.expect("Timed task context rejected").reason,
                UserReason::Fault,
                "Timed task faulted: {:?}",
                case,
            );
        } else {
            assert!(
                ipc.waits.next_deadline().is_some(),
                "Timed fixture deadlocked"
            );
            idle_until_deadline(&mut tasks, &mut ipc);
            assert!(!x86_64::instructions::interrupts::are_enabled());
            idle_returns += 1;
        }
        if matches!(case, Case::Queued | Case::CapabilityTimeout) && !reaped[0] {
            queued_wait_observed |= tasks.resources_mut(first).unwrap().wait.is_some();
        }
        for (index, id) in [first, second].into_iter().enumerate() {
            if tasks.state(id) != Ok(TaskState::Dying) {
                continue;
            }
            let resources = tasks.resources_mut(id).unwrap();
            if resources.exit_code != expected[index] {
                let _ = kryo_hal::print(format_args!(
                    "TIME: wrong exit case={:?} task={:?} expected={} actual={} rdx={:#x} r8={:#x} last_failure={:?}\n",
                    case,
                    id,
                    expected[index],
                    resources.exit_code,
                    resources.context.rdx,
                    resources.context.r8,
                    calls.last_failure,
                ));
            }
            assert_eq!(
                resources.exit_code, expected[index],
                "Timed fixture: {:?}",
                case
            );
            assert!(resources.wait.is_none());
            drop(
                tasks
                    .reap(id, |resources| resources.close(pool, region))
                    .unwrap(),
            );
            reaped[index] = true;
        }
    }
    assert_eq!(
        reaped,
        [true, true],
        "Timed fixture did not exit: {:?}",
        case
    );
    assert_eq!(ipc.waits.next_deadline(), None);
    if matches!(case, Case::Queued | Case::CapabilityTimeout) {
        assert!(queued_wait_observed, "Timed call never queued: {:?}", case);
    }
    assert_eq!(ipc.waits.next_ready(), None);
    assert_eq!(ipc.transfer_count(), 0);
    assert_eq!(
        ipc.accepted_failures,
        usize::from(matches!(case, Case::Accepted | Case::AcceptedCapability))
    );
    assert_eq!(
        ipc.transferred,
        usize::from(matches!(case, Case::AcceptedCapability))
    );
    drop(ipc);
    assert_eq!(objects.live_count(), 0);
    assert_eq!(pool.stats(), before);
    idle_returns
}

fn check_early_interrupt(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) -> usize {
    let before = pool.stats();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::new();
    let mut calls = CallStats::default();
    let sleeper = tasks
        .spawn(Resources::new(
            pool,
            direct,
            kernel,
            region,
            2,
            &code::early_interrupt_sleeper(),
        ))
        .unwrap();
    for _ in 0..32 {
        if tasks.state(sleeper) == Ok(TaskState::Blocked) {
            break;
        }
        step(&mut tasks, &mut calls, &mut ipc)
            .1
            .expect("Early-IRQ sleeper context rejected");
    }
    assert_eq!(tasks.state(sleeper), Ok(TaskState::Blocked));
    let wait = tasks.resources_mut(sleeper).unwrap().wait.unwrap();
    let deadline = ipc.waits.next_deadline().unwrap();
    assert!(ipc.waits.is_pending(wait));
    assert!(kryo_hal::monotonic_time().unwrap() < deadline);

    unsafe { kryo_hal::arm_timer_once(1) }.expect("Cannot queue early timer");
    for _ in 0..100_000 {
        if kryo_hal::timer_pending() {
            break;
        }
        core::hint::spin_loop();
    }
    assert!(
        kryo_hal::timer_pending(),
        "Early timer did not become pending"
    );
    let epoch = kryo_hal::timer_irq_epoch();
    idle_until_deadline(&mut tasks, &mut ipc);
    assert!(kryo_hal::timer_irq_epoch() > epoch);
    assert!(kryo_hal::monotonic_time().unwrap() < deadline);
    assert_eq!(tasks.state(sleeper), Ok(TaskState::Blocked));
    assert!(ipc.waits.is_pending(wait));
    assert_eq!(ipc.waits.next_deadline(), Some(deadline));

    let mut idle_returns = 1;
    for _ in 0..64 {
        if tasks.has_runnable() {
            break;
        }
        idle_until_deadline(&mut tasks, &mut ipc);
        idle_returns += 1;
    }
    assert!(tasks.has_runnable());
    assert!(kryo_hal::monotonic_time().unwrap() >= deadline);
    for _ in 0..32 {
        if tasks.state(sleeper) == Ok(TaskState::Dying) {
            break;
        }
        let (_, outcome) = step(&mut tasks, &mut calls, &mut ipc);
        assert_ne!(outcome.unwrap().reason, UserReason::Fault);
    }
    assert_eq!(tasks.state(sleeper), Ok(TaskState::Dying));
    assert_eq!(tasks.resources_mut(sleeper).unwrap().exit_code, 78);
    assert_eq!(ipc.waits.next_deadline(), None);
    assert_eq!(ipc.waits.next_ready(), None);
    drop(
        tasks
            .reap(sleeper, |resources| resources.close(pool, region))
            .unwrap(),
    );
    drop(ipc);
    assert_eq!(pool.stats(), before);
    kryo_hal::emergency_write_str("TIME: early IRQ ignored until deadline\n");
    idle_returns
}
