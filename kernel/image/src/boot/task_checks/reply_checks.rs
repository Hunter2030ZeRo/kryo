use super::{
    BootFramePool, CallStats, DirectMap, Handles, IpcRuntime, KernelMemory, KernelObject,
    KernelRegion, Objects, RefCell, Resources, Rights, Tasks, programs, step,
};
use kryo_core::ipc::Endpoint;
use kryo_core::object::ObjectKind;
use kryo_core::process::ExitReason;
use kryo_core::resource::{Budget, Limits, Resource};
use kryo_interfaces::syscall::{Call, Error};
use programs::Program;

const MAPPING: u64 = 0xc0_0000;
const PAYLOAD: u64 = 0xfedc_ba98_7654_3210;
const REQUEST: [u64; 4] = [0x8000_0000_0000_0041, 0x1234_5678_9abc_def0, 7, u64::MAX];
const RESPONSE: [u64; 4] = [0xfedc_ba98_7654_3242, 0x8000_0000_0000_0001, 0, 99];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    RoundTrip,
    One,
    Plain,
    HandleQuota,
    Expired,
    AcceptedTimeout,
}

pub(super) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    for case in [
        Case::RoundTrip,
        Case::One,
        Case::Plain,
        Case::HandleQuota,
        Case::Expired,
        Case::AcceptedTimeout,
    ] {
        run_case(case, pool, direct, memory, region);
    }
    kryo_hal::emergency_write_str(
        "REPLYCAPS: atomic grants quota timeout and retirement OK cases=6\n",
    );
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
        let limits = Limits {
            shared_pages: 4,
            private_pages: 0,
            reserved_pages: 0,
            handles: 8,
            waits: 4,
            threads: 1,
            mappings: 32,
            tables: 24,
        };
        let caller_account = Budget::new(Limits {
            handles: if case == Case::HandleQuota { 1 } else { 8 },
            ..limits
        });
        let server_account = Budget::new(limits);
        let caller = publish(
            &mut tasks,
            Resources::new(pool, direct, memory, region, 2, &caller_program(case)),
            &caller_account,
        );
        let server = publish(
            &mut tasks,
            Resources::new(pool, direct, memory, region, 7, &server_program(case)),
            &server_account,
        );
        let endpoint = objects
            .create(
                ObjectKind::Endpoint,
                KernelObject::Endpoint(RefCell::new(Endpoint::growing(server))),
            )
            .unwrap();
        for (task, rights) in [(caller, Rights::SEND), (server, Rights::RECEIVE)] {
            let mut owner = tasks.resources_mut(task).unwrap();
            owner.context.rbp = owner
                .process
                .handles
                .insert(endpoint.try_clone().unwrap(), rights)
                .unwrap()
                .as_raw();
        }
        ipc.register(endpoint);
        let mut retired = 0;
        let mut preflight = false;
        let deadline = kryo_hal::monotonic_time().unwrap() + 3_000_000_000;
        while tasks.live_count() != 0 {
            assert!(
                kryo_hal::monotonic_time().unwrap() < deadline,
                "Reply fixture stalled: {:?}",
                case
            );
            if tasks.has_runnable() {
                let (id, outcome) = step(&mut tasks, &mut calls, &mut ipc);
                let outcome = outcome.expect("Reply fixture rejected user context");
                assert_ne!(
                    outcome.reason,
                    kryo_hal::UserReason::Fault,
                    "Reply fixture fault: {:?} {:?}",
                    case,
                    outcome
                );
                if id == caller && matches!(case, Case::HandleQuota | Case::Expired) && !preflight {
                    let expected = if case == Case::HandleQuota {
                        Error::OutOfResources
                    } else {
                        Error::TimedOut
                    };
                    if calls
                        .last_failure
                        .is_some_and(|(task, _, error)| task == caller && error == expected)
                    {
                        assert_eq!(caller_account.count(Resource::Handles), 1);
                        assert_eq!(caller_account.count(Resource::Waits), 0);
                        assert_eq!(
                            tasks
                                .resources_mut(caller)
                                .unwrap()
                                .process
                                .handles
                                .reserved_count(),
                            0
                        );
                        assert!(tasks.thread(caller).unwrap().wait.is_none());
                        preflight = true;
                    }
                }
            } else {
                super::idle_until_deadline(&mut tasks, &mut ipc);
            }
            while let Some(task) = tasks.next_dying(None) {
                let owner = tasks.resources_mut(task).unwrap();
                assert_eq!(owner.exit_reason, ExitReason::Exited);
                assert_eq!(
                    owner.exit_code,
                    if task == caller { 42 } else { 43 },
                    "Reply fixture failed: {:?}",
                    case
                );
                assert_eq!(owner.process.handles.reserved_count(), 0);
                assert!(owner.wait.is_none());
                drop(
                    tasks
                        .reap(task, &mut ipc, |owner| owner.close(pool, region))
                        .unwrap(),
                );
                retired += 1;
            }
        }
        assert_eq!(retired, 2);
        assert_eq!(preflight, matches!(case, Case::HandleQuota | Case::Expired));
        assert_eq!(ipc.waits.live_count(), 0);
        assert_eq!(ipc.transfer_count(), 0);
        drop(ipc);
        assert_eq!(objects.live_count(), 0);
        for account in [&caller_account, &server_account] {
            for resource in [
                Resource::SharedPages,
                Resource::PrivatePages,
                Resource::ReservedPages,
                Resource::Handles,
                Resource::Waits,
                Resource::Threads,
            ] {
                assert_eq!(
                    account.count(resource),
                    0,
                    "Reply resource survived: {:?}",
                    case
                );
            }
        }
    }
    assert_eq!(memory.borrow().transient_heap_usage(), heap);
    assert_eq!(
        pool.stats().free + memory.borrow().retained_pages(),
        before.free + retained
    );
}

fn publish<'kernel, 'objects>(
    tasks: &mut Tasks<'kernel, 'objects>,
    mut resources: Resources<'kernel, 'objects>,
    account: &Budget,
) -> kryo_core::task::TaskId {
    resources.process.handles = Handles::budgeted(account.clone());
    resources.process.account = Some(account.clone());
    resources.thread_charge = Some(account.try_charge(Resource::Threads, 1).unwrap());
    tasks.spawn(resources).unwrap()
}

fn caller_program(case: Case) -> Program {
    let mut code = Program::new();
    if case == Case::RoundTrip {
        code.call(Call::MemoryCreate, 1, 0);
        code.expect_status(0);
        code.emit(&[0x48, 0x89, 0x13]); // retain request source at DATA
        code.emit(&[0x49, 0x89, 0xd4]); // r12=source
    }
    if matches!(case, Case::HandleQuota | Case::Expired) {
        exchange(&mut code, case == Case::Expired, 0, 2);
        code.expect_status(if case == Case::HandleQuota {
            Error::OutOfResources.status()
        } else {
            Error::TimedOut.status()
        });
        code.expect_value(0);
    }
    let capacity = match case {
        Case::RoundTrip => 2,
        Case::Plain | Case::HandleQuota => 0,
        _ => 1,
    };
    if case == Case::AcceptedTimeout {
        code.call(Call::ClockRead, 0, 0);
        code.expect_status(0);
        code.emit(&[0x48, 0x81, 0xc2]);
        code.emit(&200_000_000u32.to_le_bytes());
    }
    exchange(
        &mut code,
        case == Case::AcceptedTimeout,
        usize::from(case == Case::RoundTrip),
        capacity,
    );
    if case == Case::AcceptedTimeout {
        code.expect_status(Error::TimedOut.status());
        code.expect_value(1);
    } else {
        code.expect_status(0);
        code.expect_value(0);
        compare_message(&mut code, RESPONSE);
        code.emit(&[0x49, 0x81, 0xfe]);
        code.emit(&(capacity as u32).to_le_bytes());
        code.failure_jump();
        if capacity != 0 {
            saved(&mut code, Call::HandleInfo, 12);
            code.expect_status(0);
            code.expect_value(2);
            compare(&mut code, 8, (Rights::READ | Rights::MAP).bits());
            code.immediate(&[0x48, 0xbe], MAPPING);
            code.emit(&[0x31, 0xd2]);
            code.immediate(&[0x49, 0xb8], 0);
            saved(&mut code, Call::MemoryMap, 12);
            code.expect_status(0);
            code.immediate(&[0x48, 0xb8], MAPPING);
            code.emit(&[0x48, 0x8b, 0x00]);
            code.immediate(&[0x48, 0xb9], PAYLOAD);
            code.emit(&[0x48, 0x39, 0xc8]);
            code.failure_jump();
            code.immediate(&[0x48, 0xbe], Rights::WRITE.bits());
            saved(&mut code, Call::Duplicate, 12);
            code.expect_status(Error::PermissionDenied.status());
            code.call(Call::MemoryUnmap, MAPPING, 0);
            code.expect_status(0);
            saved(&mut code, Call::Close, 12);
            code.expect_status(0);
        }
        if capacity == 2 {
            saved(&mut code, Call::HandleInfo, 13);
            code.expect_status(0);
            code.expect_value(2);
            compare(&mut code, 8, Rights::READ.bits());
            saved(&mut code, Call::Close, 13);
            code.expect_status(0);
        }
    }
    if case == Case::RoundTrip {
        code.emit(&[0x4c, 0x8b, 0x3b]); // original request source remains valid
        code.call_saved_handle(Call::Close);
        code.expect_status(0);
    }
    code.call(Call::Exit, 42, 0);
    code.finish()
}

fn server_program(case: Case) -> Program {
    let mut code = Program::new();
    code.immediate(&[0x48, 0xbe], u64::from(case == Case::RoundTrip));
    endpoint_call(&mut code, Call::Receive);
    code.expect_status(0);
    compare_message(&mut code, REQUEST);
    code.emit(&[0x49, 0x89, 0xd7]); // r15=reply token
    if case == Case::RoundTrip {
        saved(&mut code, Call::Close, 12);
        code.expect_status(0);
    }
    for register in [12, 13] {
        code.call(Call::MemoryCreate, 1, 0);
        code.expect_status(0);
        code.emit(&[0x49, 0x89, if register == 12 { 0xd4 } else { 0xd5 }]);
    }
    code.immediate(&[0x48, 0xbe], MAPPING);
    code.emit(&[0x31, 0xd2]);
    code.immediate(&[0x49, 0xb8], 1);
    saved(&mut code, Call::MemoryMap, 12);
    code.expect_status(0);
    code.immediate(&[0x48, 0xb8], MAPPING);
    code.immediate(&[0x48, 0xb9], PAYLOAD);
    code.emit(&[0x48, 0x89, 0x08]);
    // Keep the token outside grant registers through every retry.
    code.emit(&[0x4c, 0x89, 0x3b]);
    if case == Case::AcceptedTimeout {
        code.call(Call::ClockRead, 0, 0);
        code.emit(&[0x48, 0x89, 0xd7, 0x48, 0x81, 0xc7]);
        code.emit(&400_000_000u32.to_le_bytes());
        code.emit(&[0xb8]);
        code.emit(&(Call::SleepUntil as u32).to_le_bytes());
        code.emit(&[0xcd, 0x80]);
        code.expect_status(0);
        reply(
            &mut code,
            Call::ReplyCaps,
            1,
            (Rights::READ | Rights::MAP).bits(),
        );
        code.expect_status(Error::InvalidReply.status());
    } else {
        reply(&mut code, Call::ReplyCaps, 3, Rights::READ.bits());
        code.expect_status(Error::InvalidArgument.status());
        let count = if case == Case::RoundTrip { 2 } else { 1 };
        if matches!(case, Case::Plain | Case::HandleQuota) {
            reply(&mut code, Call::ReplyCaps, count, Rights::READ.bits());
            code.expect_status(Error::OutOfResources.status());
            reply(&mut code, Call::Reply, 0, 0);
        } else {
            reply(&mut code, Call::ReplyCaps, count, Rights::EXECUTE.bits());
            code.expect_status(Error::PermissionDenied.status());
            if case == Case::RoundTrip {
                code.emit(&[0x4c, 0x89, 0x6b, 0x08, 0x45, 0x31, 0xed]);
                reply(
                    &mut code,
                    Call::ReplyCaps,
                    count,
                    (Rights::READ | Rights::MAP).bits(),
                );
                code.expect_status(Error::InvalidHandle.status());
                code.emit(&[0x4c, 0x8b, 0x6b, 0x08]);
            }
            reply(
                &mut code,
                Call::ReplyCaps,
                count,
                (Rights::READ | Rights::MAP).bits(),
            );
        }
        code.expect_status(0);
        reply(&mut code, Call::ReplyCaps, count, Rights::READ.bits());
        code.expect_status(Error::InvalidReply.status());
    }
    for register in [12, 13] {
        saved(&mut code, Call::Close, register);
        code.expect_status(0);
    }
    code.call(Call::MemoryUnmap, MAPPING, 0);
    code.expect_status(0);
    code.call(Call::Exit, 43, 0);
    code.finish()
}

fn exchange(code: &mut Program, timed: bool, requests: usize, capacity: usize) {
    code.immediate(&[0x48, 0xbe], requests as u64);
    code.immediate(&[0x48, 0xb9], capacity as u64);
    message(code, REQUEST);
    code.immediate(&[0x49, 0xbe], Rights::READ.bits());
    code.immediate(&[0x49, 0xbf], 0);
    endpoint_call(
        code,
        if timed {
            Call::CallExchangeUntil
        } else {
            Call::CallExchange
        },
    );
}

fn reply(code: &mut Program, call: Call, count: usize, rights: u64) {
    code.emit(&[0x48, 0x8b, 0x33]);
    code.immediate(&[0x48, 0xba], count as u64);
    message(code, RESPONSE);
    code.immediate(&[0x49, 0xbe], rights);
    code.immediate(&[0x49, 0xbf], Rights::READ.bits());
    endpoint_call(code, call);
}

fn message(code: &mut Program, words: [u64; 4]) {
    for (register, word) in words.into_iter().enumerate() {
        code.immediate(&[0x49, 0xb8 + register as u8], word);
    }
}

fn compare_message(code: &mut Program, words: [u64; 4]) {
    for (register, word) in words.into_iter().enumerate() {
        compare(code, 8 + register as u8, word);
    }
}

fn compare(code: &mut Program, register: u8, value: u64) {
    code.immediate(&[0x48, 0xb8], value);
    code.emit(&[0x49, 0x39, 0xc0 + register - 8]);
    code.failure_jump();
}

fn endpoint_call(code: &mut Program, call: Call) {
    code.emit(&[0x48, 0x89, 0xef, 0xb8]);
    code.emit(&(call as u32).to_le_bytes());
    code.emit(&[0xcd, 0x80]);
}

fn saved(code: &mut Program, call: Call, register: u8) {
    code.emit(&[0x4c, 0x89, 0xc7 + (register - 8) * 8, 0xb8]);
    code.emit(&(call as u32).to_le_bytes());
    code.emit(&[0xcd, 0x80]);
}
