use super::{
    BootFramePool, CallStats, DirectMap, IpcRuntime, KernelMemory, KernelObject, KernelRegion,
    Objects, Program, RefCell, Resources, TaskState, Tasks, idle_until_deadline, programs,
};
use crate::runtime::{Object, step_with_region};
use kryo_core::mm::vm::Permissions;
use kryo_core::object::ObjectKind;
use kryo_core::process::{ExitReason, ExitStatus};
use kryo_core::task::TaskId;
use kryo_hal::{PagingError, UserReason};
use kryo_interfaces::syscall::{Call, Error};

const CHILD_OFFSET: usize = 1024;
const CHILD: u64 = programs::CODE + CHILD_OFFSET as u64;
const READ_ONLY: u64 = 0x70_0000;
const CHILD_GUARD: u64 = 0x81_fff8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    InvalidEntry,
    SelfJoin,
    Capacity,
    GuardFault,
    ChildProcessExit,
    MainThreadExit,
    TimedJoin,
    RetireRetry,
    ObjectsFull,
}

pub(super) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    for case in [
        Case::InvalidEntry,
        Case::SelfJoin,
        Case::Capacity,
        Case::GuardFault,
        Case::ChildProcessExit,
        Case::MainThreadExit,
        Case::TimedJoin,
        Case::RetireRetry,
        Case::ObjectsFull,
    ] {
        run_case(case, pool, direct, memory, region);
    }
    kryo_hal::emergency_write_str("THREAD: lifecycle and isolation OK cases=9\n");
}

fn run_case(
    case: Case,
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let objects = Objects::new();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::with_objects(&objects);
    let mut stats = CallStats::default();
    let parent = prepare_parent(case, pool, direct, memory, region, &mut tasks);
    let process = tasks.process_for(parent).unwrap();
    let peer = if case == Case::GuardFault {
        Some(prepare_peer(pool, direct, memory, region, &mut tasks))
    } else {
        None
    };
    let retained = retain_objects(case, &objects);
    let allocated = pool.stats();
    let mut terminal = None;
    let mut parent_reaped = false;
    let mut child_reaped = false;
    let mut fault_peer_ticks = None;
    let mut rollback_checked = false;
    let mut join_blocked = false;
    let mut retirement_checked = false;

    for _ in 0..256 {
        if tasks.process_thread_count(process).is_err() {
            if let Some(peer) = peer {
                let ticks = tasks.thread_mut(peer).unwrap().ticks;
                if ticks >= fault_peer_ticks.expect("Missing isolated guard fault") + 2 {
                    stop_peer(&mut tasks, &mut ipc, peer, pool, region);
                    break;
                }
            } else {
                break;
            }
        }
        if tasks.has_runnable() {
            let (task, outcome) = step_with_region(&mut tasks, &mut stats, &mut ipc, region);
            let outcome = outcome.expect("Thread fixture context rejected");
            if outcome.reason == UserReason::Fault {
                assert_eq!(case, Case::GuardFault);
                assert_ne!(task, parent);
                assert_eq!(outcome.vector, 14);
                assert_eq!(outcome.error, 6);
                assert_eq!(outcome.address, CHILD_GUARD);
                let peer = peer.unwrap();
                assert_eq!(tasks.state(peer), Ok(TaskState::Runnable));
                fault_peer_ticks = Some(tasks.thread_mut(peer).unwrap().ticks);
            }
            if task == parent && tasks.state(parent) == Ok(TaskState::Blocked) {
                join_blocked = true;
            }
            if case == Case::ObjectsFull
                && task == parent
                && outcome.reason == UserReason::Call
                && !rollback_checked
            {
                check_rollback(&mut tasks, parent, region);
                assert_eq!(pool.stats(), allocated);
                assert_eq!(objects.live_count(), 8);
                rollback_checked = true;
            }
        } else {
            assert!(
                ipc.waits.next_deadline().is_some(),
                "Thread fixture deadlocked: {:?}",
                case
            );
            idle_until_deadline(&mut tasks, &mut ipc);
        }

        terminal = tasks.terminal(process).or(terminal);
        let mut dying = [None; 4];
        for (slot, task) in dying.iter_mut().zip(tasks.ids().filter(|task| {
            tasks.state(*task) == Ok(TaskState::Dying) && tasks.process_for(*task) == Some(process)
        })) {
            *slot = Some(task);
        }
        for task in dying.into_iter().flatten() {
            check_exit(case, &mut tasks, parent, task);
            if task == parent {
                parent_reaped = true;
                if case == Case::MainThreadExit {
                    assert_eq!(tasks.terminal(process), None);
                    assert!(tasks.process_thread_count(process).unwrap() > 1);
                }
            } else {
                child_reaped = true;
                if case == Case::RetireRetry {
                    assert!(!retirement_checked);
                    check_retirement_retry(&mut tasks, &mut ipc, &mut stats, parent, task, region);
                    retirement_checked = true;
                    continue;
                }
            }
            reap(&mut tasks, &mut ipc, task, pool, region);
        }
    }

    let expected = expectation(case);
    assert_eq!(
        terminal,
        Some(expected),
        "Thread process result: {:?}",
        case
    );
    assert!(parent_reaped, "Parent did not retire: {:?}", case);
    let created = usize::from(!matches!(case, Case::InvalidEntry | Case::ObjectsFull));
    assert_eq!(child_reaped, created != 0);
    assert_eq!(stats.threads_created, created);
    assert_eq!(rollback_checked, case == Case::ObjectsFull);
    assert_eq!(retirement_checked, case == Case::RetireRetry);
    if matches!(
        case,
        Case::GuardFault | Case::ChildProcessExit | Case::TimedJoin
    ) {
        assert!(join_blocked, "Parent never blocked in Join: {:?}", case);
    }
    assert_eq!(tasks.live_count(), 0);
    assert_eq!(tasks.process_count(), 0);
    assert_eq!(ipc.waits.next_ready(), None);
    assert_eq!(ipc.waits.next_deadline(), None);
    assert_eq!(ipc.transfer_count(), 0);
    drop(retained);
    drop(ipc);
    assert_eq!(objects.live_count(), 0);
    assert_eq!(pool.stats(), before);
}

#[inline(never)]
fn prepare_parent<'kernel, 'objects>(
    case: Case,
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    tasks: &mut Tasks<'kernel, 'objects>,
) -> TaskId {
    let mut resources = Resources::new(pool, direct, memory, region, 2, &program(case));
    if case == Case::InvalidEntry {
        resources
            .process
            .space
            .map_page(READ_ONLY, Permissions::ReadOnly, &[0x90])
            .unwrap();
    }
    tasks.spawn(resources).unwrap()
}

#[inline(never)]
fn prepare_peer<'kernel, 'objects>(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    tasks: &mut Tasks<'kernel, 'objects>,
) -> TaskId {
    tasks
        .spawn(Resources::new(
            pool,
            direct,
            memory,
            region,
            17,
            &programs::counter(),
        ))
        .unwrap()
}

fn retain_objects<'objects>(
    case: Case,
    objects: &'objects Objects,
) -> [Option<Object<'objects>>; 8] {
    let mut retained = [const { None }; 8];
    if case == Case::ObjectsFull {
        for slot in &mut retained {
            *slot = Some(
                objects
                    .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
                    .unwrap(),
            );
        }
    }
    retained
}

fn check_rollback(tasks: &mut Tasks<'_, '_>, parent: TaskId, region: &KernelRegion) {
    let resources = tasks.resources_mut(parent).unwrap();
    assert_eq!(resources.context.rax, Error::OutOfResources.status());
    assert_eq!(resources.context.rdx, 0);
    assert_eq!(resources.process.handles.live_count(), 0);
    assert_eq!(resources.process.handles.reserved_count(), 0);
    assert!(resources.process.pending_thread.is_none());
    for page in crate::runtime::user_pages(1) {
        assert_eq!(resources.process.space.permissions(page), None);
    }
    for slot in 6..=10 {
        assert_eq!(region.permissions(slot), Err(PagingError::NotMapped));
    }
}

fn check_retirement_retry<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    parent: TaskId,
    child: TaskId,
    region: &mut KernelRegion,
) {
    let process = tasks.process_for(child).unwrap();
    let (pool, memory) = {
        let thread = tasks.thread_mut(child).unwrap();
        (thread.pool, thread.memory)
    };
    let before = pool.stats();
    memory.borrow_mut().fail_next_restore();
    assert!(matches!(
        tasks.reap(child, ipc, |resources| resources.close(pool, region)),
        Err(kryo_core::task::ReapError::Cleanup(_))
    ));
    assert_eq!(tasks.state(child), Ok(TaskState::Dying));
    assert_eq!(tasks.process_thread_count(process), Ok(2));
    let first_page = crate::runtime::user_pages(1)[0];
    assert_eq!(
        tasks
            .process_mut(process)
            .unwrap()
            .space
            .permissions(first_page),
        None
    );
    assert!(tasks.thread_mut(child).unwrap().user_stack_owned);
    // Only the four kernel-stack frames retired. The unpublished first user
    // page still retains its backing owner after restore_frame failed.
    assert_eq!(pool.stats().free, before.free + 4);

    let mut refused = false;
    for _ in 0..8 {
        let (task, outcome) = step_with_region(tasks, stats, ipc, region);
        assert_eq!(task, parent);
        let outcome = outcome.expect("Parent rejected during thread cleanup retry");
        assert_ne!(outcome.reason, UserReason::Fault);
        if outcome.reason == UserReason::Call
            && tasks.thread_mut(parent).unwrap().context.rax == Error::OutOfResources.status()
        {
            refused = true;
            break;
        }
    }
    assert!(
        refused,
        "Retained thread slot was reused before cleanup succeeded"
    );
    assert_eq!(tasks.process_thread_count(process), Ok(2));
    assert_eq!(pool.stats().free, before.free + 4);
    reap(tasks, ipc, child, pool, region);
    assert_eq!(tasks.process_thread_count(process), Ok(1));
    assert!(tasks.state(child).is_err());
    assert_eq!(pool.stats().free, before.free + 9);
    for page in crate::runtime::user_pages(1) {
        assert_eq!(
            tasks.process_mut(process).unwrap().space.permissions(page),
            None
        );
    }
}

fn check_exit(case: Case, tasks: &mut Tasks<'_, '_>, parent: TaskId, task: TaskId) {
    let expected = if task == parent {
        if case == Case::MainThreadExit {
            exited(0)
        } else {
            expectation(case)
        }
    } else {
        match case {
            Case::SelfJoin => exited(81),
            Case::Capacity => stopped(83),
            Case::TimedJoin => stopped(85),
            Case::RetireRetry => exited(7),
            _ => expectation(case),
        }
    };
    let resources = tasks.resources_mut(task).unwrap();
    assert_eq!(
        resources.exit_code, expected.code,
        "Thread result: {:?}",
        case
    );
    assert_eq!(resources.exit_reason, expected.reason);
    assert!(resources.wait.is_none());
    if task == parent && matches!(case, Case::GuardFault | Case::ChildProcessExit) {
        let mut bytes = [0; 8];
        resources
            .process
            .space
            .read_page(programs::DATA, 16, &mut bytes)
            .unwrap();
        assert_eq!(
            u64::from_le_bytes(bytes),
            0,
            "Killed parent resumed its join"
        );
    }
}

#[inline(never)]
fn reap<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    task: TaskId,
    pool: &BootFramePool,
    region: &mut KernelRegion,
) {
    drop(
        tasks
            .reap(task, ipc, |resources| resources.close(pool, region))
            .unwrap(),
    );
}

fn stop_peer<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    peer: TaskId,
    pool: &BootFramePool,
    region: &mut KernelRegion,
) {
    let thread = tasks.thread_mut(peer).unwrap();
    assert!(thread.ticks > 0);
    thread.exit_code = 90;
    thread.exit_reason = ExitReason::Stopped;
    tasks.stop(peer).unwrap();
    ipc.settle(tasks);
    reap(tasks, ipc, peer, pool, region);
}

fn expectation(case: Case) -> ExitStatus {
    match case {
        Case::InvalidEntry => exited(80),
        Case::SelfJoin => exited(82),
        Case::Capacity => exited(84),
        Case::GuardFault => ExitStatus {
            reason: ExitReason::Fault,
            code: 0x100 + 14,
        },
        Case::ChildProcessExit => exited(42),
        Case::MainThreadExit => exited(9),
        Case::TimedJoin => exited(86),
        Case::RetireRetry => exited(89),
        Case::ObjectsFull => exited(88),
    }
}

fn exited(code: u64) -> ExitStatus {
    ExitStatus {
        reason: ExitReason::Exited,
        code,
    }
}
fn stopped(code: u64) -> ExitStatus {
    ExitStatus {
        reason: ExitReason::Stopped,
        code,
    }
}

fn program(case: Case) -> Program {
    let mut parent = Program::new();
    match case {
        Case::InvalidEntry => {
            for entry in [0, 0xffff_ffff_8000_0000, programs::CODE + 4096, READ_ONLY] {
                parent.call(Call::ThreadCreate, entry, programs::DATA);
                parent.expect_status(Error::InvalidArgument.status());
                parent.expect_value(0);
            }
            parent.call(Call::Exit, 80, 0);
        }
        Case::ObjectsFull => {
            parent.call(Call::ThreadCreate, CHILD, programs::DATA);
            parent.expect_status(Error::OutOfResources.status());
            parent.expect_value(0);
            parent.call(Call::Exit, 88, 0);
        }
        _ => {
            if case == Case::MainThreadExit {
                parent.emit(&[0x48, 0xc7, 0x43, 0x08, 0xbc, 0x0a, 0x00, 0x00]);
            }
            parent.call(Call::ThreadCreate, CHILD, programs::DATA);
            parent.expect_status(0);
            parent.emit(&[0x49, 0x89, 0xd7]); // Retain the opaque thread handle in R15.
            match case {
                Case::SelfJoin => {
                    parent.emit(&[0x48, 0x89, 0x13]);
                    join(&mut parent, 81, ExitReason::Exited);
                    parent.call(Call::Exit, 82, 0);
                }
                Case::Capacity => {
                    parent.call(Call::ThreadCreate, CHILD, programs::DATA);
                    parent.expect_status(Error::OutOfResources.status());
                    parent.expect_value(0);
                    stop_and_join(&mut parent, 83);
                    parent.call(Call::Exit, 84, 0);
                }
                Case::GuardFault | Case::ChildProcessExit => {
                    parent.emit(&[0x48, 0xc7, 0x03, 0x01, 0x00, 0x00, 0x00]);
                    parent.call_saved_handle(Call::ThreadJoin);
                    parent.emit(&[0x48, 0xc7, 0x43, 0x10, 0x01, 0x00, 0x00, 0x00]);
                    parent.call(Call::Exit, 255, 0);
                }
                Case::MainThreadExit => {
                    parent.emit(&[0x48, 0xc7, 0x03, 0x01, 0x00, 0x00, 0x00]);
                    parent.call(Call::ThreadExit, 0, 0);
                }
                Case::TimedJoin => {
                    parent.call(Call::ClockRead, 0, 0);
                    parent.expect_status(0);
                    parent.emit(&[0x48, 0x81, 0xc2]);
                    parent.emit(&20_000_000u32.to_le_bytes());
                    parent.call_saved_handle(Call::ThreadJoinUntil);
                    parent.expect_status(Error::TimedOut.status());
                    parent.expect_value(0);
                    stop_and_join(&mut parent, 85);
                    parent.call(Call::Exit, 86, 0);
                }
                Case::RetireRetry => {
                    join(&mut parent, 7, ExitReason::Exited);
                    parent.call(Call::ThreadCreate, CHILD, programs::DATA);
                    parent.expect_status(Error::OutOfResources.status());
                    parent.expect_value(0);
                    parent.call(Call::Exit, 89, 0);
                }
                _ => unreachable!(),
            }
        }
    }
    let mut parent = parent.finish();
    assert!(parent.as_bytes().len() <= CHILD_OFFSET);
    while parent.as_bytes().len() < CHILD_OFFSET {
        parent.emit(&[0x90]);
    }
    parent.emit(child(case).finish().as_bytes());
    parent
}

fn child(case: Case) -> Program {
    let mut child = Program::new();
    child.emit(&[0x48, 0x89, 0xfb]); // RDI is the parent's shared data address.
    match case {
        Case::RetireRetry => child.call(Call::ThreadExit, 7, 0),
        Case::SelfJoin => {
            child.emit(&[0x4c, 0x8b, 0x3b, 0x4d, 0x85, 0xff, 0x74, 0xf8]);
            child.call_saved_handle(Call::ThreadJoin);
            child.expect_status(Error::PermissionDenied.status());
            child.call(Call::ThreadExit, 81, 0);
        }
        Case::GuardFault | Case::ChildProcessExit | Case::MainThreadExit => {
            child.emit(&[0x48, 0x83, 0x3b, 0x01, 0x75, 0xfa]);
            // One handoff ensures the parent has reached Join or ThreadExit
            // even if it was preempted immediately after publishing the flag.
            child.call(Call::Yield, 0, 0);
            child.expect_status(0);
            match case {
                Case::GuardFault => {
                    child.immediate(&[0x48, 0xb8], CHILD_GUARD);
                    child.emit(&[0x48, 0xc7, 0x00, 0x01, 0x00, 0x00, 0x00]);
                }
                Case::ChildProcessExit => child.call(Call::Exit, 42, 0),
                Case::MainThreadExit => {
                    child.emit(&[0x48, 0x81, 0x7b, 0x08]);
                    child.emit(&0xabcu32.to_le_bytes());
                    child.failure_jump();
                    child.call(Call::ThreadExit, 9, 0);
                }
                _ => unreachable!(),
            }
        }
        _ => child.emit(&[0x48, 0xff, 0x43, 0x08, 0xeb, 0xfa]),
    }
    child
}

fn join(code: &mut Program, value: u32, reason: ExitReason) {
    code.call_saved_handle(Call::ThreadJoin);
    code.expect_status(0);
    code.expect_value(value);
    code.emit(&[0x49, 0x81, 0xf8]);
    code.emit(&(reason as u32).to_le_bytes());
    code.failure_jump();
}

fn stop_and_join(code: &mut Program, status: u32) {
    code.immediate(&[0x48, 0xbe], u64::from(status));
    code.call_saved_handle(Call::TaskStop);
    code.expect_status(0);
    join(code, status, ExitReason::Stopped);
    code.call_saved_handle(Call::Close);
    code.expect_status(0);
}
