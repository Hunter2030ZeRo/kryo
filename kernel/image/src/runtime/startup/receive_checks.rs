use crate::runtime::{
    BootFramePool, CallStats, Handles, IpcRuntime, KernelMemory, KernelObject, KernelRegion,
    Objects, RefCell, Tasks,
};
use kryo_core::capability::{Handle, Rights};
use kryo_core::ipc::Endpoint;
use kryo_core::object::ObjectKind;
use kryo_core::sched::{ReturnReason, TaskState};
use kryo_core::task::TaskId;
use kryo_hal::{DirectMap, UserContext};
use kryo_interfaces::syscall::{Call, Error};

const POLL_NS: u64 = 1_000_000;
const MESSAGE: u64 = 0x7265_6c61_7469_7665;

pub(crate) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let retained = memory.borrow().retained_pages();
    {
        let image =
            crate::boot::module_image("kryo-worker").expect("Missing receive fixture image");
        let objects = Objects::growing();
        let mut tasks = Tasks::growing();
        let mut ipc = IpcRuntime::growing(&objects);
        let resources = super::super::launch::prepare(pool, direct, memory, region, 2, image)
            .unwrap_or_else(|_| panic!("Cannot prepare receive fixture"));
        let caller = tasks.spawn(resources).unwrap();
        let endpoint = objects
            .create(
                ObjectKind::Endpoint,
                KernelObject::Endpoint(RefCell::new(Endpoint::growing(caller))),
            )
            .unwrap();
        ipc.try_register(endpoint.try_clone().unwrap()).unwrap();
        let (full, reduced, wrong) = {
            let resources = tasks.resources_mut(caller).unwrap();
            let handles = &mut resources.process.handles;
            let full = handles
                .insert(endpoint.try_clone().unwrap(), Rights::RECEIVE | Rights::DUP)
                .unwrap();
            let reduced = handles.duplicate(full, Rights::DUP).unwrap();
            let wrong = handles
                .insert(
                    objects
                        .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
                        .unwrap(),
                    Rights::WRITE,
                )
                .unwrap();
            (full, reduced, wrong)
        };
        let mut calls = CallStats::default();
        for (handle, duration, expected) in [
            (full, 0, Error::TimedOut),
            (full, u64::MAX, Error::InvalidArgument),
            (reduced, 0, Error::PermissionDenied),
            (wrong, 0, Error::WrongType),
        ] {
            let context = invoke(
                caller,
                Call::ReceiveFor,
                handle,
                duration,
                &mut tasks,
                &mut calls,
                &mut ipc,
            );
            assert_eq!(context.rax, expected.status());
            assert_eq!(context.rdx, 0);
            assert_eq!(tasks.state(caller), Ok(TaskState::Runnable));
            assert!(tasks.thread(caller).unwrap().wait.is_none());
            assert_eq!(
                tasks
                    .resources_mut(caller)
                    .unwrap()
                    .process
                    .handles
                    .reserved_count(),
                0
            );
            assert_eq!(ipc.waits.next_deadline(), None);
            assert_eq!(ipc.waits.next_ready(), None);
        }
        assert_eq!(calls.expired_receive_admissions, 0);
        let old_clock = invoke(
            caller,
            Call::ClockRead,
            Handle::INVALID,
            0,
            &mut tasks,
            &mut calls,
            &mut ipc,
        )
        .rdx;
        while kryo_hal::monotonic_time().unwrap() <= old_clock + POLL_NS {
            core::hint::spin_loop();
        }
        let mut resources = super::super::launch::prepare(pool, direct, memory, region, 7, image)
            .unwrap_or_else(|_| panic!("Cannot prepare queued sender"));
        resources.process.handles = Handles::growing();
        let send = resources
            .process
            .handles
            .insert(endpoint.try_clone().unwrap(), Rights::SEND)
            .unwrap();
        resources.context.r8 = MESSAGE;
        let peer = tasks.spawn(resources).unwrap();
        // Advance the fixture's serialized scheduler without executing user code.
        assert_eq!(tasks.dispatch().unwrap(), Some(caller));
        tasks.returned(caller, ReturnReason::Yielded).unwrap();
        let send_deadline = kryo_hal::monotonic_time()
            .unwrap()
            .checked_add(1_000_000_000)
            .unwrap();
        invoke(
            peer,
            Call::SendUntil,
            send,
            send_deadline,
            &mut tasks,
            &mut calls,
            &mut ipc,
        );
        assert_eq!(tasks.state(peer), Ok(TaskState::Blocked));
        let expired = invoke(
            caller,
            Call::ReceiveUntil,
            full,
            old_clock + POLL_NS,
            &mut tasks,
            &mut calls,
            &mut ipc,
        );
        assert_eq!(expired.rax, Error::TimedOut.status());
        assert_eq!(tasks.state(peer), Ok(TaskState::Blocked));
        let received = invoke(
            caller,
            Call::ReceiveFor,
            full,
            POLL_NS,
            &mut tasks,
            &mut calls,
            &mut ipc,
        );
        assert_eq!(received.rax, 0);
        assert_eq!(received.r8, MESSAGE);
        assert_eq!(tasks.state(peer), Ok(TaskState::Runnable));
        assert_eq!(calls.expired_receive_admissions, 1);
        tasks.stop(peer).unwrap();
        ipc.settle(&mut tasks);
        drop(
            tasks
                .reap(peer, &mut ipc, |resources| resources.close(pool, region))
                .unwrap(),
        );
        invoke(
            caller,
            Call::ReceiveFor,
            full,
            POLL_NS,
            &mut tasks,
            &mut calls,
            &mut ipc,
        );
        assert_eq!(tasks.state(caller), Ok(TaskState::Blocked));
        let deadline = ipc
            .waits
            .next_deadline()
            .expect("Relative receive did not publish its deadline");
        while tasks.state(caller) == Ok(TaskState::Blocked) {
            super::super::idle_until_deadline(&mut tasks, &mut ipc);
        }
        assert!(kryo_hal::monotonic_time().unwrap() >= deadline);
        assert_eq!(tasks.state(caller), Ok(TaskState::Runnable));
        assert_eq!(
            tasks.thread(caller).unwrap().context.rax,
            Error::TimedOut.status()
        );
        assert_eq!(calls.expired_receive_admissions, 1);
        assert_eq!(ipc.waits.next_deadline(), None);
        tasks.stop(caller).unwrap();
        ipc.settle(&mut tasks);
        drop(
            tasks
                .reap(caller, &mut ipc, |resources| resources.close(pool, region))
                .unwrap(),
        );
        ipc.settle(&mut tasks);
        drop(endpoint);
        assert_eq!(objects.live_count(), 0);
    }
    assert_eq!(
        pool.stats().free,
        before.free - (memory.borrow().retained_pages() - retained)
    );
    kryo_hal::emergency_write_str("RECEIVE: relative admission and absolute expiry OK\n");
}

fn invoke<'objects>(
    caller: TaskId,
    call: Call,
    handle: Handle,
    duration: u64,
    tasks: &mut Tasks<'_, 'objects>,
    calls: &mut CallStats,
    ipc: &mut IpcRuntime<'objects>,
) -> UserContext {
    {
        let mut resources = tasks.resources_mut(caller).unwrap();
        resources.context.rax = call as u64;
        resources.context.rdi = handle.as_raw();
        resources.context.rsi = 0;
        resources.context.rdx = duration;
    }
    assert_eq!(tasks.dispatch().unwrap(), Some(caller));
    tasks.returned(caller, ReturnReason::Call).unwrap();
    super::super::native::dispatch(caller, tasks, calls, ipc, None);
    ipc.settle(tasks);
    tasks.thread(caller).unwrap().context
}
