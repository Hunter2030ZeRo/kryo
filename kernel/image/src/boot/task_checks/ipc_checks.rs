use super::ipc_programs as code;
use super::{
    BootFramePool, CallStats, DirectMap, IpcRuntime, KernelMemory, KernelObject, KernelRegion,
    Objects, RefCell, Resources, Rights, TaskState, Tasks, step,
};
use kryo_core::ipc::{Endpoint, Notification};
use kryo_core::object::ObjectKind;
use kryo_hal::{IdleCause, UserReason};

#[derive(Clone, Copy, Debug)]
enum Case {
    Rpc,
    CancelQueued,
    CancelAccepted,
    OwnerQueued,
    OwnerAccepted,
    CloseQueued,
    NotifyPending,
    NotifyWaiting,
}

pub(super) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    for case in [
        Case::Rpc,
        Case::CancelQueued,
        Case::CancelAccepted,
        Case::OwnerQueued,
        Case::OwnerAccepted,
        Case::CloseQueued,
        Case::NotifyPending,
        Case::NotifyWaiting,
    ] {
        run_case(case, pool, direct, kernel, region);
    }
    kryo_hal::emergency_write_str("IPC: rendezvous and lifetime OK cases=8\n");

    unsafe { kryo_hal::arm_timer_once(100_000) }.expect("Cannot queue idle timer");
    for _ in 0..100_000 {
        if kryo_hal::timer_pending() {
            break;
        }
        core::hint::spin_loop();
    }
    assert!(kryo_hal::timer_pending());
    let wake = kryo_hal::idle_until_interrupt().expect("Cannot idle with pending IRQ");
    assert_eq!(wake.cause, IdleCause::Timer);
    assert!(wake.after > wake.before);
    assert!(!x86_64::instructions::interrupts::are_enabled());
    unsafe { kryo_hal::stop_timer() }.unwrap();
    kryo_hal::emergency_write_str("IDLE: halt and wake OK cases=3\n");
}

fn run_case(
    case: Case,
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let objects = Objects::new();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::new();
    let mut calls = CallStats::default();
    let (first_code, second_code, expected) = match case {
        Case::Rpc => (code::rpc_server(0), code::rpc_client(0), [12, 11]),
        Case::CancelQueued => (
            code::cancelled_client(0, 0),
            code::closer_or_canceller(0, true),
            [14, 15],
        ),
        Case::CancelAccepted => (
            code::cancelling_server(0),
            code::cancelled_client(0, 1),
            [19, 14],
        ),
        Case::OwnerQueued => (
            code::peer_closed_client(0),
            code::exiting_queued_server(),
            [16, 17],
        ),
        Case::OwnerAccepted => (
            code::exiting_server(0, true),
            code::peer_closed_client(0),
            [17, 16],
        ),
        Case::CloseQueued => (
            code::peer_closed_client(0),
            code::closer_or_canceller(0, false),
            [16, 15],
        ),
        Case::NotifyPending => (code::signaller(0), code::double_waiter(0, 18), [13, 18]),
        Case::NotifyWaiting => (
            code::waiter(0, 20),
            code::exiting_server(0, false),
            [20, 17],
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

    let notification = if matches!(case, Case::NotifyPending | Case::NotifyWaiting) {
        let owner = if matches!(case, Case::NotifyPending) {
            second
        } else {
            first
        };
        let object = objects
            .create(
                ObjectKind::Notification,
                KernelObject::Notification(RefCell::new(Notification::new(owner))),
            )
            .unwrap();
        let mut consumer = tasks.resources_mut(owner).unwrap();
        consumer.context.rbp = consumer
            .process
            .handles
            .insert(object.try_clone().unwrap(), Rights::WAIT)
            .unwrap()
            .as_raw();
        if matches!(case, Case::NotifyPending) {
            let control = objects
                .create(ObjectKind::TaskControl, KernelObject::TaskControl(first))
                .unwrap();
            // R12 lets the consumer observe both real Signals and producer Exit
            // before its first Wait, regardless of intervening preemption.
            consumer.context.r12 = consumer
                .process
                .handles
                .insert(control, Rights::INSPECT)
                .unwrap()
                .as_raw();
            let mut producer = tasks.resources_mut(first).unwrap();
            producer.context.rbp = producer
                .process
                .handles
                .insert(object.try_clone().unwrap(), Rights::SIGNAL)
                .unwrap()
                .as_raw();
        }
        ipc.register(object.try_clone().unwrap());
        Some(object)
    } else {
        let owner = if matches!(
            case,
            Case::CancelQueued | Case::OwnerQueued | Case::CloseQueued
        ) {
            second
        } else {
            first
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
            .insert(
                object.try_clone().unwrap(),
                Rights::RECEIVE | Rights::CONTROL,
            )
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
        if matches!(
            case,
            Case::CancelQueued | Case::CancelAccepted | Case::CloseQueued | Case::OwnerQueued
        ) {
            let control = objects
                .create(ObjectKind::TaskControl, KernelObject::TaskControl(sender))
                .unwrap();
            let mut receiver = tasks.resources_mut(owner).unwrap();
            let rights = match case {
                Case::CancelQueued => Rights::CONTROL | Rights::INSPECT,
                Case::CancelAccepted => Rights::CONTROL,
                _ => Rights::INSPECT,
            };
            let handle = receiver
                .process
                .handles
                .insert(control, rights)
                .unwrap()
                .as_raw();
            match case {
                Case::CancelQueued => {
                    receiver.context.rbp = handle;
                    receiver.context.r15 = handle;
                }
                Case::CancelAccepted => receiver.context.rbx = handle,
                Case::CloseQueued | Case::OwnerQueued => receiver.context.r15 = handle,
                _ => unreachable!(),
            }
        }
        None
    };

    let mut reaped = [false; 2];
    let mut idle_count = 0;
    for _ in 0..128 {
        if tasks.live_count() == 0 {
            break;
        }
        if !tasks.has_runnable() {
            let object = notification.as_ref().expect("IPC fixture deadlocked");
            assert_eq!(idle_count, 0);
            // No runnable work was observed with IRQs disabled. The timer IRQ
            // records an event; object completion runs here after the IRQ returns.
            unsafe { kryo_hal::arm_timer_once(100_000) }.unwrap();
            let wake = kryo_hal::idle_until_interrupt().expect("Cannot idle blocked tasks");
            assert_eq!(wake.cause, IdleCause::Timer);
            assert!(wake.after > wake.before);
            assert!(!x86_64::instructions::interrupts::are_enabled());
            unsafe { kryo_hal::stop_timer() }.unwrap();
            object.with(|object| {
                let KernelObject::Notification(notification) = object else {
                    unreachable!()
                };
                notification.borrow_mut().signal(&mut ipc.waits).unwrap();
            });
            ipc.settle(&mut tasks);
            idle_count += 1;
            continue;
        }
        let (_, outcome) = step(&mut tasks, &mut calls, &mut ipc);
        let outcome = outcome.expect("IPC task context rejected");
        assert_ne!(outcome.reason, UserReason::Fault);
        for (index, id) in [first, second].into_iter().enumerate() {
            if tasks.state(id) != Ok(TaskState::Dying) {
                continue;
            }
            let resources = tasks.resources_mut(id).unwrap();
            if resources.exit_code != expected[index] {
                let _ = kryo_hal::print(format_args!(
                    "IPC failure case={:?} task={:?} expected={} actual={} last={:?}\n",
                    case, id, expected[index], resources.exit_code, calls.last_failure
                ));
            }
            assert_eq!(resources.exit_code, expected[index]);
            assert!(resources.wait.is_none());
            drop(
                tasks
                    .reap(id, |resources| resources.close(pool, region))
                    .unwrap(),
            );
            reaped[index] = true;
        }
    }
    assert_eq!(reaped, [true, true]);
    assert_eq!(tasks.live_count(), 0);
    assert_eq!(ipc.waits.next_ready(), None);
    assert_eq!(
        ipc.accepted_failures,
        usize::from(matches!(case, Case::CancelAccepted | Case::OwnerAccepted))
    );
    if matches!(case, Case::NotifyPending | Case::NotifyWaiting) {
        if idle_count != 1 {
            let _ = kryo_hal::print(format_args!(
                "IPC: notification idle mismatch case={:?} count={}\n",
                case, idle_count,
            ));
        }
        assert_eq!(idle_count, 1);
    }
    if matches!(case, Case::Rpc) {
        assert_eq!(calls.denied, 1);
    }
    drop(notification);
    drop(ipc);
    assert_eq!(objects.live_count(), 0);
    assert_eq!(pool.stats(), before);
}
