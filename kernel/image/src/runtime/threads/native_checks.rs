use crate::boot::memory::BootFramePool;
use crate::boot::vm_backend::KernelMemory;
use crate::runtime::{CallStats, IpcRuntime, KernelObject, KernelRegion, Object, Objects, Tasks};
use core::cell::RefCell;
use kryo_core::capability::{Handle, Rights};
use kryo_core::mm::vm::{Permissions, VmError};
use kryo_core::object::ObjectKind;
use kryo_core::process::{ExitReason, ExitStatus};
use kryo_hal::{DirectMap, UserContext};
use kryo_interfaces::syscall::{Call, Error};

const STACK: u64 = 0x1_0000_1000;
const STACK_BYTES: u64 = 4 * 4096;
const TLS: u64 = 0x1_0010_0000;

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
        super::create(parent, legacy, &mut tasks, &mut ipc, &mut stats, region).unwrap();
        let managed = tasks
            .ids()
            .find(|id| *id != parent && *id != child)
            .unwrap();
        assert_eq!(tasks.thread(managed).unwrap().user_slot, 1);
        assert!(tasks.thread(managed).unwrap().user_stack_owned);

        tasks.thread_mut(child).unwrap().exit_code = 7;
        tasks.stop(child).unwrap();
        ipc.settle(&mut tasks);
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
                        reason: ExitReason::Exited,
                        code: 7,
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
