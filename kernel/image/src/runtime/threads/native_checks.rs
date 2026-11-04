use crate::boot::memory::BootFramePool;
use crate::boot::vm_backend::KernelMemory;
use crate::runtime::{CallStats, IpcRuntime, KernelObject, KernelRegion, Object, Objects, Tasks};
use core::cell::RefCell;
use kryo_core::capability::{Handle, Rights};
use kryo_core::mm::vm::{Permissions, VmError};
use kryo_core::object::ObjectKind;
use kryo_core::process::{ExitReason, ExitStatus};
use kryo_core::resource::{Budget, Limits, Resource};
use kryo_hal::{DirectMap, UserContext};
use kryo_interfaces::syscall::{Call, Error};

const STACK: u64 = 0x1_0000_1000;
const STACK_BYTES: u64 = 4 * 4096;
const TLS: u64 = 0x1_0010_0000;
const EXIT_CODE: u64 = 0xfedc_ba98_7654_3210;

pub(crate) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let retained = memory.borrow().retained_pages();
    {
        let objects = Objects::new();
        let mut tasks = Tasks::growing();
        let mut ipc = IpcRuntime::with_objects(&objects);
        let image = crate::boot::module_image("kryo-native-smoke").unwrap();
        let slot = region.find_stack(pool, direct).unwrap();
        let resources = crate::runtime::launch::prepare(pool, direct, memory, region, slot, image)
            .unwrap_or_else(|mut failure| {
                failure.pending.close(pool, region);
                panic!("Cannot prepare runtime-owned thread fixture");
            });
        let parent = tasks
            .spawn(resources)
            .unwrap_or_else(|_| panic!("Cannot publish runtime-owned thread fixture"));
        let process = tasks.process_for(parent).unwrap();
        let mut stats = CallStats::default();
        {
            let owner = tasks.process_mut(process).unwrap();
            for page in (STACK..STACK + STACK_BYTES).step_by(4096) {
                owner
                    .space
                    .map_page(page, Permissions::ReadWrite, &[])
                    .unwrap();
            }
            owner
                .space
                .map_page(TLS, Permissions::ReadWrite, &[])
                .unwrap();
        }
        let mut context = tasks.thread(parent).unwrap().context;
        context.rdi = 1;
        context.rsi = context.rip;
        context.rdx = 77;
        context.r8 = STACK;
        context.r9 = STACK_BYTES;
        context.r10 = TLS;
        context.r11 = 4096;
        context.r12 = TLS + 256;
        context.r13 = 4096;
        context.r14 = 0;
        admission(&mut tasks, &mut ipc, &mut stats, parent, context, region);
        rollback(
            &mut tasks, &mut ipc, &mut stats, &objects, parent, context, region,
        );

        let control =
            super::create_native(parent, context, &mut tasks, &mut ipc, &mut stats, region)
                .unwrap();
        let child = tasks.ids().find(|id| *id != parent).unwrap();
        {
            let thread = tasks.thread(child).unwrap();
            assert_eq!(thread.user_slot, usize::MAX);
            assert!(!thread.user_stack_owned);
            assert!(thread.execution.is_some());
            assert_eq!(thread.context.rdi, 77);
            assert_eq!(thread.context.rsi, TLS);
            assert_eq!(thread.context.rsp, STACK + STACK_BYTES);
            assert_eq!(thread.context.fs_base, TLS + 256);
        }
        tls_updates(&mut tasks, &mut ipc, &mut stats, child, region);
        assert_eq!(
            super::create_native(parent, context, &mut tasks, &mut ipc, &mut stats, region),
            Err(Error::Busy)
        );
        {
            let owner = tasks.process_mut(process).unwrap();
            assert!(matches!(owner.space.unmap_page(STACK), Err(VmError::InUse)));
            assert!(matches!(
                owner
                    .space
                    .map_page(STACK - 4096, Permissions::ReadWrite, &[]),
                Err(VmError::InUse)
            ));
        }

        let mut legacy = context;
        legacy.rdi = context.rsi;
        legacy.rsi = 0;
        let managed_control =
            super::create(parent, legacy, &mut tasks, &mut ipc, &mut stats, region).unwrap();
        let managed = tasks
            .ids()
            .find(|id| *id != parent && *id != child)
            .unwrap();
        assert_eq!(tasks.thread(managed).unwrap().user_slot, 1);
        assert!(tasks.thread(managed).unwrap().user_stack_owned);

        tasks.thread_mut(child).unwrap().exit_code = EXIT_CODE;
        tasks.thread_mut(child).unwrap().exit_reason = ExitReason::Stopped;
        tasks.stop(child).unwrap();
        ipc.settle(&mut tasks);
        terminal_join(
            &mut tasks,
            &mut ipc,
            &mut stats,
            parent,
            control,
            managed_control,
            region,
        );
        {
            let owner = tasks.process_mut(process).unwrap();
            owner.space.ensure_unleased(STACK, STACK_BYTES).unwrap();
            let object = owner
                .handles
                .lookup(
                    Handle::from_raw(control),
                    ObjectKind::ThreadControl,
                    Rights::WAIT,
                )
                .unwrap();
            object.with(|object| {
                assert_eq!(
                    crate::runtime::with_exit(object, |exit| exit.result()),
                    Some(Some(ExitStatus {
                        reason: ExitReason::Stopped,
                        code: EXIT_CODE,
                    }))
                );
            });
            for page in (STACK..STACK + STACK_BYTES).step_by(4096) {
                owner.space.unmap_page(page).unwrap();
            }
            owner.space.unmap_page(TLS).unwrap();
            owner
                .space
                .map_page(STACK, Permissions::ReadWrite, &[0x5a])
                .unwrap();
        }
        drop(
            tasks
                .reap(child, &mut ipc, |owner| owner.close(pool, region))
                .unwrap(),
        );
        {
            let owner = tasks.process_mut(process).unwrap();
            let mut value = [0];
            owner.space.read_page(STACK, 0, &mut value).unwrap();
            assert_eq!(value, [0x5a]);
        }
        tasks
            .stop_process(
                process,
                ExitStatus {
                    reason: ExitReason::Stopped,
                    code: 0,
                },
            )
            .unwrap();
        ipc.settle(&mut tasks);
        while let Some(task) = tasks.next_dying(None) {
            drop(
                tasks
                    .reap(task, &mut ipc, |owner| owner.close(pool, region))
                    .unwrap(),
            );
        }
        assert_eq!(tasks.live_count(), 0);
        assert_eq!(objects.live_count(), 0);
        assert_eq!(stats.threads_created, 2);
    }
    assert_eq!(pool.stats().total, before.total);
    assert_eq!(
        pool.stats().free + memory.borrow().retained_pages(),
        before.free + retained
    );
    kryo_hal::emergency_write_str("THREAD: runtime-owned admission rollback and lifetime OK\n");
}

fn terminal_join<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    parent: kryo_core::task::TaskId,
    terminal: u64,
    active: u64,
    region: &mut KernelRegion,
) {
    let process = tasks.process_for(parent).unwrap();
    let account = Budget::new(Limits {
        shared_pages: usize::MAX,
        private_pages: usize::MAX,
        reserved_pages: usize::MAX,
        handles: usize::MAX,
        waits: 0,
        threads: usize::MAX,
        mappings: usize::MAX,
        tables: usize::MAX,
    });
    let original_account = tasks
        .process_mut(process)
        .unwrap()
        .account
        .replace(account.clone());
    let original = tasks.thread(parent).unwrap().context;
    let future = kryo_hal::monotonic_time().unwrap() + 1_000_000_000;
    for (call, handle, deadline, status, value) in [
        (
            Call::ThreadJoin,
            active,
            0,
            Error::OutOfResources.status(),
            0,
        ),
        (
            Call::ThreadJoinUntil,
            terminal,
            0,
            Error::TimedOut.status(),
            0,
        ),
        (Call::ThreadJoin, terminal, 0, 0, EXIT_CODE),
        (Call::ThreadJoinUntil, terminal, future, 0, EXIT_CODE),
    ] {
        wait_call(
            tasks,
            ipc,
            stats,
            parent,
            region,
            (call, handle, deadline),
            (status, value),
        );
    }
    terminal_process_wait(tasks, ipc, stats, parent, region);
    assert_eq!(account.count(Resource::Waits), 0);
    tasks.thread_mut(parent).unwrap().context = original;
    tasks.process_mut(process).unwrap().account = original_account;
}

fn terminal_process_wait<'kernel, 'objects>(
    tasks: &mut Tasks<'kernel, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    parent: kryo_core::task::TaskId,
    region: &mut KernelRegion,
) {
    let process = tasks.process_for(parent).unwrap();
    let (pool, direct, memory) = {
        let thread = tasks.thread(parent).unwrap();
        (thread.pool, thread.direct, thread.memory)
    };
    let objects = ipc.arena.unwrap();
    let new_control = || {
        objects
            .create(
                ObjectKind::ProcessControl,
                KernelObject::ProcessControl(RefCell::new(crate::runtime::ProcessControl::new())),
            )
            .unwrap()
    };
    let control = new_control();
    let image = crate::boot::module_image("kryo-native-smoke").unwrap();
    let slot = region.find_stack(pool, direct).unwrap();
    let mut resources = crate::runtime::launch::prepare(pool, direct, memory, region, slot, image)
        .unwrap_or_else(|mut failure| {
            failure.pending.close(pool, region);
            panic!("Cannot prepare terminal process wait fixture");
        });
    resources.process.control = Some(control.try_clone().unwrap());
    let target = tasks.spawn(resources).unwrap();
    let target_process = tasks.process_for(target).unwrap();
    let terminal = tasks
        .process_mut(process)
        .unwrap()
        .handles
        .insert(control, Rights::WAIT)
        .unwrap()
        .as_raw();
    let control = new_control();
    crate::runtime::process::bind(&control, parent, process);
    let own = tasks
        .process_mut(process)
        .unwrap()
        .handles
        .insert(control, Rights::WAIT)
        .unwrap()
        .as_raw();
    let unbound = tasks
        .process_mut(process)
        .unwrap()
        .handles
        .insert(new_control(), Rights::WAIT)
        .unwrap()
        .as_raw();
    for (handle, error) in [
        (terminal, Error::OutOfResources),
        (own, Error::PermissionDenied),
        (unbound, Error::PeerClosed),
    ] {
        wait_call(
            tasks,
            ipc,
            stats,
            parent,
            region,
            (Call::ProcessWait, handle, 0),
            (error.status(), 0),
        );
    }
    tasks
        .stop_process(
            target_process,
            ExitStatus {
                reason: ExitReason::Stopped,
                code: EXIT_CODE,
            },
        )
        .unwrap();
    ipc.settle(tasks);
    let future = kryo_hal::monotonic_time().unwrap() + 1_000_000_000;
    for (call, deadline, status, value) in [
        (Call::ProcessWaitUntil, 0, Error::TimedOut.status(), 0),
        (Call::ProcessWait, 0, 0, EXIT_CODE),
        (Call::ProcessWaitUntil, future, 0, EXIT_CODE),
    ] {
        wait_call(
            tasks,
            ipc,
            stats,
            parent,
            region,
            (call, terminal, deadline),
            (status, value),
        );
    }
    drop(
        tasks
            .reap(target, ipc, |owner| owner.close(pool, region))
            .unwrap(),
    );
    wait_call(
        tasks,
        ipc,
        stats,
        parent,
        region,
        (Call::ProcessWait, terminal, 0),
        (0, EXIT_CODE),
    );
}

fn wait_call<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    parent: kryo_core::task::TaskId,
    region: &mut KernelRegion,
    call: (Call, u64, u64),
    expected: (u64, u64),
) {
    let waits = ipc.waits.live_count();
    let context = &mut tasks.thread_mut(parent).unwrap().context;
    context.rax = call.0 as u64;
    context.rdi = call.1;
    context.rdx = call.2;
    context.r8 = u64::MAX;
    crate::runtime::native::dispatch(parent, tasks, stats, ipc, Some(region));
    let thread = tasks.thread(parent).unwrap();
    assert_eq!(thread.context.rax, expected.0);
    assert_eq!(thread.context.rdx, expected.1);
    if expected.0 == 0 {
        assert_eq!(thread.context.r8, ExitReason::Stopped as u64);
    }
    assert!(thread.wait.is_none());
    assert_eq!(ipc.waits.live_count(), waits);
    assert_eq!(
        tasks
            .resources_mut(parent)
            .unwrap()
            .process
            .account
            .as_ref()
            .unwrap()
            .count(Resource::Waits),
        0,
    );
}

fn admission<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    parent: kryo_core::task::TaskId,
    valid: UserContext,
    region: &mut KernelRegion,
) {
    let mut invalid = [valid; 8];
    invalid[0].rdi = 2;
    invalid[1].r14 = 1;
    invalid[2].rsi = TLS;
    invalid[3].r8 += 8;
    invalid[4].r12 = TLS + 4096;
    invalid[5].r11 = 0;
    invalid[6].r10 = crate::runtime::USER_TLS[0];
    invalid[6].r12 = invalid[6].r10;
    invalid[7].r13 = 1;
    for context in invalid {
        assert_eq!(
            super::create_native(parent, context, tasks, ipc, stats, region),
            Err(Error::InvalidArgument)
        );
    }
    assert_eq!(stats.threads_created, 0);
}

fn tls_updates<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    child: kryo_core::task::TaskId,
    region: &mut KernelRegion,
) {
    let original = tasks.thread(child).unwrap().context;
    for address in [0, TLS - 16, TLS + 4096] {
        let context = &mut tasks.thread_mut(child).unwrap().context;
        context.rax = Call::SetTls as u64;
        context.rdi = address;
        crate::runtime::native::dispatch(child, tasks, stats, ipc, Some(region));
        let context = tasks.thread(child).unwrap().context;
        assert_eq!(context.rax, Error::InvalidArgument.status());
        assert_eq!(context.fs_base, original.fs_base);
    }
    for address in [TLS, TLS + 16, original.fs_base] {
        let context = &mut tasks.thread_mut(child).unwrap().context;
        context.rax = Call::SetTls as u64;
        context.rdi = address;
        crate::runtime::native::dispatch(child, tasks, stats, ipc, Some(region));
        let context = tasks.thread(child).unwrap().context;
        assert_eq!(context.rax, 0);
        assert_eq!(context.fs_base, address);
    }
}

fn rollback<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    objects: &'objects Objects,
    parent: kryo_core::task::TaskId,
    context: UserContext,
    region: &mut KernelRegion,
) {
    let mut retained: [Option<Object<'objects>>; 8] = [const { None }; 8];
    for slot in &mut retained {
        *slot = Some(
            objects
                .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
                .unwrap(),
        );
    }
    assert_eq!(
        super::create_native(parent, context, tasks, ipc, stats, region),
        Err(Error::OutOfResources)
    );
    let owner = tasks.resources_mut(parent).unwrap();
    owner
        .process
        .space
        .ensure_unleased(STACK, STACK_BYTES)
        .unwrap();
    owner.process.space.ensure_unleased(TLS, 4096).unwrap();
    assert_eq!(
        owner.process.space.permissions(STACK),
        Some(Permissions::ReadWrite)
    );
    assert_eq!(
        owner.process.space.permissions(TLS),
        Some(Permissions::ReadWrite)
    );
    assert!(owner.process.pending_thread.is_none());
    assert_eq!(owner.process.handles.reserved_count(), 0);
    assert_eq!(owner.process.handles.live_count(), 0);
    assert_eq!(stats.threads_created, 0);
    drop(retained);
}
