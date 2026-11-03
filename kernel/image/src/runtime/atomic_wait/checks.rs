use crate::boot::memory::BootFramePool;
use crate::boot::vm_backend::KernelMemory;
use crate::runtime::{CallStats, IpcRuntime, KernelObject, KernelRegion, Object, Objects, Tasks};
use core::cell::RefCell;
use kryo_core::ipc::{Completion, Delivery, IpcError};
use kryo_core::mm::vm::Permissions;
use kryo_core::object::ObjectKind;
use kryo_core::process::{ExitReason, ExitStatus};
use kryo_core::sched::{ReturnReason, TaskState};
use kryo_core::sync::WaitId;
use kryo_core::task::TaskId;
use kryo_hal::DirectMap;
use kryo_interfaces::syscall::{Call, Error};

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
        let mut tasks = Tasks::new();
        let mut ipc = IpcRuntime::with_objects(&objects);
        let image = crate::boot::module_image("kryo-native-smoke").unwrap();
        let slot = region.find_stack(pool, direct).unwrap();
        let resources = crate::runtime::launch::prepare(pool, direct, memory, region, slot, image)
            .unwrap_or_else(|mut failure| {
                failure.pending.close(pool, region);
                panic!("Cannot prepare private wait retirement fixture");
            });
        let parent = tasks
            .spawn(resources)
            .unwrap_or_else(|_| panic!("Cannot publish wait fixture"));
        let mut stats = CallStats::default();

        pending_thread(
            &mut tasks, &mut ipc, &mut stats, &objects, parent, memory, region,
        );
        stale_wake(&mut tasks, &mut ipc, &mut stats, parent, region);
        unbounded_idle(&mut tasks, &mut ipc, parent);

        let process = tasks.process_for(parent).unwrap();
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
        drop(
            tasks
                .reap(parent, &mut ipc, |owner| owner.close(pool, region))
                .unwrap(),
        );
        assert_eq!(tasks.live_count(), 0);
        assert!(ipc.words.is_empty());
        assert_eq!(ipc.waits.live_count(), 0);
        assert_eq!(objects.live_count(), 0);
    }
    assert_eq!(pool.stats().total, before.total);
    assert_eq!(
        pool.stats().free + memory.borrow().retained_pages() - retained,
        before.free
    );
}

fn pending_thread<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    objects: &'objects Objects,
    parent: TaskId,
    memory: &RefCell<KernelMemory>,
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
    let mut context = tasks.thread(parent).unwrap().context;
    context.rdi = context.rip;
    context.rsi = 0;
    memory.borrow_mut().fail_next_restore();
    assert_eq!(
        crate::runtime::threads::create(parent, context, tasks, ipc, stats, region),
        Err(Error::OutOfResources)
    );
    let page = crate::runtime::user_pages(1)[1];
    {
        let owner = tasks.resources_mut(parent).unwrap();
        assert!(owner.process.pending_thread.is_some());
        assert_eq!(
            owner.process.space.permissions(page),
            Some(Permissions::ReadWrite)
        );
    }
    let wait = queue_word(tasks, ipc, parent, page, None);
    context.rdi = 0;
    assert_eq!(
        crate::runtime::threads::create(parent, context, tasks, ipc, stats, region),
        Err(Error::InvalidArgument)
    );
    assert!(ipc.words.is_empty());
    assert!(matches!(
        ipc.waits.take(wait),
        Ok(Completion::Failed {
            error: IpcError::Cancelled,
            delivery: Delivery::NotDelivered,
        })
    ));
    let owner = tasks.resources_mut(parent).unwrap();
    assert!(owner.process.pending_thread.is_none());
    for page in crate::runtime::user_pages(1) {
        assert_eq!(owner.process.space.permissions(page), None);
    }
    assert_eq!(stats.threads_created, 0);
    drop(retained);
    assert_eq!(objects.live_count(), 0);
    kryo_hal::emergency_write_str("WAIT: pending thread retirement cancels word\n");
}

fn stale_wake<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    parent: TaskId,
    region: &mut KernelRegion,
) {
    let word = crate::runtime::USER_TLS[0];
    let wait = queue_word(
        tasks,
        ipc,
        parent,
        word,
        Some(kryo_hal::monotonic_time().unwrap()),
    );
    let context = &mut tasks.thread_mut(parent).unwrap().context;
    context.rax = Call::Wake as u64;
    context.rdi = word;
    context.rsi = 1;
    crate::runtime::native::dispatch(parent, tasks, stats, ipc, Some(region));
    let context = tasks.thread(parent).unwrap().context;
    assert_eq!((context.rax, context.rdx), (0, 0));
    assert!(ipc.words.is_empty());
    assert!(matches!(
        ipc.waits.take(wait),
        Ok(Completion::Failed {
            error: IpcError::TimedOut,
            delivery: Delivery::NotDelivered,
        })
    ));
    kryo_hal::emergency_write_str("WAIT: expired word precedes wake admission\n");
}

fn unbounded_idle<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    parent: TaskId,
) {
    assert_eq!(tasks.dispatch(), Ok(Some(parent)));
    tasks.returned(parent, ReturnReason::Call).unwrap();
    let word = crate::runtime::USER_TLS[0];
    let wait = queue_word(tasks, ipc, parent, word, None);
    tasks.thread_mut(parent).unwrap().wait = Some(wait);
    assert_eq!(tasks.state(parent), Ok(TaskState::Blocked));
    assert!(!tasks.has_runnable());
    assert_eq!(ipc.waits.next_deadline(), None);

    // This kernel-owned interrupt is independent of the untimed user wait.
    let epoch = kryo_hal::timer_irq_epoch();
    let deadline = kryo_hal::monotonic_time().unwrap() + 1_000_000;
    unsafe { kryo_hal::arm_deadline(deadline) }.unwrap();
    loop {
        crate::runtime::idle_for_events(tasks, ipc);
        assert_eq!(ipc.words.len(), 1);
        assert!(ipc.waits.is_pending(wait));
        assert!(!tasks.has_runnable());
        if kryo_hal::timer_irq_epoch() > epoch {
            break;
        }
        assert!(kryo_hal::monotonic_time().unwrap() < deadline + 10_000_000);
    }
    unsafe { kryo_hal::stop_timer() }.unwrap();
    ipc.cancel(wait).unwrap();
    ipc.settle(tasks);
    assert_eq!(tasks.state(parent), Ok(TaskState::Runnable));
    assert_eq!(
        tasks.thread(parent).unwrap().context.rax,
        Error::Cancelled.status()
    );
    assert!(ipc.words.is_empty());
    kryo_hal::emergency_write_str("WAIT: unbounded word idle preserves registration\n");
}

fn queue_word(
    tasks: &mut Tasks<'_, '_>,
    ipc: &mut IpcRuntime<'_>,
    task: TaskId,
    address: u64,
    deadline: Option<u64>,
) -> WaitId {
    let key = tasks
        .resources_mut(task)
        .unwrap()
        .process
        .space
        .private_word_key(address)
        .unwrap();
    ipc.words.prepare(ipc.words.len() + 1).unwrap();
    let wait = ipc.begin_wait(task, deadline).unwrap();
    ipc.words.push(key, wait).unwrap();
    ipc.waits.park(wait).unwrap();
    wait
}
