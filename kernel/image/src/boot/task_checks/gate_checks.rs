use super::*;
use kryo_core::ipc::Endpoint;
use kryo_interfaces::syscall::{Call, Error};

const LABEL: u64 = 0xfedc_ba98_7654_3210;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Send,
    Reply,
    Timeout,
}

pub(super) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    crate::runtime::ipc_runtime::gates::checks::run();
    for case in [Case::Send, Case::Reply, Case::Timeout] {
        run_case(case, pool, direct, memory, region);
    }
    process_death(pool, direct, memory, region);
    let _ = kryo_hal::print(format_args!(
        "GATE: layout endpoint={} object={}\n",
        core::mem::size_of::<Endpoint<2, 2>>(),
        core::mem::size_of::<KernelObject>()
    ));
    kryo_hal::emergency_write_str("GATE: labels lifetime delegation quota and retirement OK\n");
    kryo_hal::emergency_write_str("GATE: queued send call and accepted timeout loans OK cases=3\n");
}

fn run_case(
    case: Case,
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let retained = memory.borrow().retained_pages();
    let heap = memory.borrow().transient_heap_usage();
    {
        let objects = Objects::growing();
        let mut tasks = Tasks::growing();
        let mut ipc = IpcRuntime::growing(&objects);
        let mut calls = CallStats::default();
        let caller = tasks
            .spawn(Resources::new(
                pool,
                direct,
                memory,
                region,
                2,
                &client(case),
            ))
            .unwrap();
        let server = tasks
            .spawn(Resources::new(
                pool,
                direct,
                memory,
                region,
                7,
                &server(case),
            ))
            .unwrap();
        let endpoint = objects
            .create(
                ObjectKind::Endpoint,
                KernelObject::Endpoint(RefCell::new(Endpoint::growing(server))),
            )
            .unwrap();
        ipc.register(endpoint.try_clone().unwrap());
        let handle = {
            let mut resources = tasks.resources_mut(caller).unwrap();
            let handle = ipc
                .create_gate(
                    server,
                    endpoint.try_clone().unwrap(),
                    LABEL,
                    &mut resources.process.handles,
                )
                .unwrap();
            resources.context.rbp = handle.as_raw();
            if case == Case::Timeout {
                resources.context.rdx = kryo_hal::monotonic_time().unwrap() + 100_000_000;
            }
            handle
        };
        {
            let mut resources = tasks.resources_mut(server).unwrap();
            resources.context.rbp = resources
                .process
                .handles
                .insert(
                    endpoint.try_clone().unwrap(),
                    Rights::RECEIVE | Rights::CONTROL,
                )
                .unwrap()
                .as_raw();
        }
        let (id, outcome) = step(&mut tasks, &mut calls, &mut ipc);
        assert_eq!(id, caller);
        assert_ne!(outcome.unwrap().reason, UserReason::Fault);
        assert_eq!(tasks.state(caller), Ok(TaskState::Blocked));
        // Model a sibling closing the source handle while the syscall remains queued.
        tasks
            .resources_mut(caller)
            .unwrap()
            .process
            .handles
            .close(handle)
            .unwrap();
        ipc.settle(&mut tasks);
        assert_eq!(ipc.gate_count(), 1);
        let deadline = kryo_hal::monotonic_time().unwrap() + 3_000_000_000;
        let mut retired = 0;
        while tasks.live_count() != 0 {
            assert!(
                kryo_hal::monotonic_time().unwrap() < deadline,
                "Gate fixture stalled {case:?}"
            );
            if tasks.has_runnable() {
                let (_, outcome) = step(&mut tasks, &mut calls, &mut ipc);
                assert_ne!(outcome.unwrap().reason, UserReason::Fault);
            } else {
                idle_until_deadline(&mut tasks, &mut ipc);
            }
            for id in [caller, server] {
                if tasks.state(id) == Ok(TaskState::Dying) {
                    assert_eq!(
                        tasks.resources_mut(id).unwrap().exit_code,
                        77,
                        "Gate fixture failed {case:?}"
                    );
                    drop(
                        tasks
                            .reap(id, &mut ipc, |resources| resources.close(pool, region))
                            .unwrap(),
                    );
                    retired += 1;
                }
            }
        }
        assert_eq!(retired, 2);
        assert_eq!(ipc.gate_count(), 0);
        assert_eq!(ipc.waits.live_count(), 0);
        assert_eq!(ipc.transfer_count(), 0);
        drop(endpoint);
        ipc.settle(&mut tasks);
        assert_eq!(objects.live_count(), 0);
    }
    assert_eq!(memory.borrow().transient_heap_usage(), heap);
    assert_eq!(pool.stats().total, before.total);
    assert_eq!(
        pool.stats().free + memory.borrow().retained_pages(),
        before.free + retained
    );
}

fn process_death(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let retained = memory.borrow().retained_pages();
    let heap = memory.borrow().transient_heap_usage();
    {
        let objects = Objects::growing();
        let mut tasks = Tasks::growing();
        let mut ipc = IpcRuntime::growing(&objects);
        let mut calls = CallStats::default();
        let mut receiver = Program::new();
        receiver.immediate(&[0x48, 0xbe], 0);
        invoke(&mut receiver, Call::Receive);
        receiver.expect_status(0);
        receiver.expect_value(0);
        expect(&mut receiver, &[0x48, 0x39, 0xc1], LABEL);
        expect(&mut receiver, &[0x49, 0x39, 0xc7], 1);
        expect(&mut receiver, &[0x49, 0x39, 0xc0], 0);
        receiver.call(Call::Exit, 77, 0);
        let receiver = receiver.finish();
        let mut sender = Program::new();
        sender.call(Call::Exit, 77, 0);
        let sender = sender.finish();
        let server = tasks
            .spawn(Resources::new(pool, direct, memory, region, 2, &receiver))
            .unwrap();
        let client = tasks
            .spawn(Resources::new(pool, direct, memory, region, 7, &sender))
            .unwrap();
        let root = objects
            .create(
                ObjectKind::Endpoint,
                KernelObject::Endpoint(RefCell::new(Endpoint::growing(server))),
            )
            .unwrap();
        ipc.register(root.try_clone().unwrap());
        {
            let resources = tasks.resources_mut(client).unwrap();
            ipc.create_gate(
                server,
                root.try_clone().unwrap(),
                LABEL,
                &mut resources.process.handles,
            )
            .unwrap();
        }
        {
            let mut resources = tasks.resources_mut(server).unwrap();
            resources.context.rbp = resources
                .process
                .handles
                .insert(root.try_clone().unwrap(), Rights::RECEIVE)
                .unwrap()
                .as_raw();
        }
        assert_eq!(step(&mut tasks, &mut calls, &mut ipc).0, server);
        assert_eq!(tasks.state(server), Ok(TaskState::Blocked));
        assert_eq!(step(&mut tasks, &mut calls, &mut ipc).0, client);
        assert_eq!(tasks.state(client), Ok(TaskState::Dying));
        assert!(!tasks.has_runnable());
        assert_eq!(ipc.waits.next_deadline(), None);
        assert!(ipc.words.is_empty());
        assert_eq!(ipc.gate_count(), 1);
        drop(
            tasks
                .reap(client, &mut ipc, |resources| resources.close(pool, region))
                .unwrap(),
        );
        assert_eq!(
            tasks.state(server),
            Ok(TaskState::Runnable),
            "Final process handles must settle before an idle decision"
        );
        assert_eq!(ipc.gate_count(), 0);
        for _ in 0..4 {
            if tasks.state(server) == Ok(TaskState::Dying) {
                break;
            }
            assert_eq!(step(&mut tasks, &mut calls, &mut ipc).0, server);
        }
        assert_eq!(tasks.resources_mut(server).unwrap().exit_code, 77);
        drop(
            tasks
                .reap(server, &mut ipc, |resources| resources.close(pool, region))
                .unwrap(),
        );
        assert_eq!(tasks.live_count(), 0);
        assert_eq!(ipc.waits.live_count(), 0);
        drop(root);
        ipc.settle(&mut tasks);
        assert_eq!(objects.live_count(), 0);
    }
    assert_eq!(memory.borrow().transient_heap_usage(), heap);
    assert_eq!(pool.stats().total, before.total);
    assert_eq!(
        pool.stats().free + memory.borrow().retained_pages(),
        before.free + retained
    );
    kryo_hal::emergency_write_str(
        "GATE: final process handle retirement wakes receiver without polling\n",
    );
}

fn invoke(code: &mut Program, call: Call) {
    code.emit(&[0xb8]);
    code.emit(&(call as u32).to_le_bytes());
    code.emit(&[0x48, 0x89, 0xef, 0xcd, 0x80]); // mov rdi,rbp; int80
}
fn expect(code: &mut Program, compare: &[u8], value: u64) {
    code.immediate(&[0x48, 0xb8], value);
    code.emit(compare);
    code.failure_jump();
}
fn client(case: Case) -> Program {
    let mut code = Program::new();
    code.immediate(&[0x49, 0xb8], 41);
    code.immediate(&[0x48, 0xbe], 0);
    code.immediate(&[0x48, 0xb9], u64::MAX);
    code.immediate(&[0x49, 0xbf], 1); // Forged event input cannot authenticate a message.
    invoke(
        &mut code,
        match case {
            Case::Send => Call::Send,
            Case::Reply => Call::Call,
            Case::Timeout => Call::CallUntil,
        },
    );
    code.expect_status(if case == Case::Timeout {
        Error::TimedOut.status()
    } else {
        0
    });
    if case == Case::Timeout {
        code.expect_value(1);
    }
    if case == Case::Reply {
        expect(&mut code, &[0x49, 0x39, 0xc0], 42);
    }
    code.call(Call::Exit, 77, 0);
    code.finish()
}
fn server(case: Case) -> Program {
    let mut code = Program::new();
    code.immediate(&[0x48, 0xbe], 0);
    invoke(&mut code, Call::Receive);
    code.expect_status(0);
    expect(&mut code, &[0x48, 0x39, 0xc1], LABEL); // cmp rcx,rax
    expect(&mut code, &[0x49, 0x39, 0xc7], 0); // cmp r15,rax
    expect(&mut code, &[0x49, 0x39, 0xc0], 41);
    code.emit(&[0x48, 0x89, 0xd3]); // mov rbx,rdx: accepted token
    if case == Case::Reply {
        code.emit(&[0x48, 0x89, 0xde]); // mov rsi,rbx
        code.immediate(&[0x49, 0xb8], 42);
        invoke(&mut code, Call::Reply);
        code.expect_status(0);
    }
    code.immediate(&[0x48, 0xbe], 0);
    invoke(&mut code, Call::Receive);
    code.expect_status(0);
    code.expect_value(0);
    expect(&mut code, &[0x48, 0x39, 0xc1], LABEL);
    expect(&mut code, &[0x49, 0x39, 0xc7], 1);
    for compare in [
        [0x49, 0x39, 0xc0],
        [0x49, 0x39, 0xc1],
        [0x49, 0x39, 0xc2],
        [0x49, 0x39, 0xc3],
        [0x49, 0x39, 0xc4],
        [0x49, 0x39, 0xc5],
        [0x49, 0x39, 0xc6],
    ] {
        expect(&mut code, &compare, 0);
    }
    if case == Case::Timeout {
        code.emit(&[0x48, 0x89, 0xde]);
        invoke(&mut code, Call::Reply);
        code.expect_status(Error::InvalidReply.status());
    }
    code.call(Call::Exit, 77, 0);
    code.finish()
}
