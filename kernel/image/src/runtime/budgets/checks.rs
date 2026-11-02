use crate::runtime::{
    self, BootFramePool, CallStats, Handles, IpcRuntime, KernelMemory, KernelObject, KernelRegion,
    Objects, RefCell, SharedLease, Tasks,
};
use kryo_core::capability::{Handle, Rights};
use kryo_core::ipc::{Completion, Message};
use kryo_core::object::ObjectKind;
use kryo_core::process::ProcessTable;
use kryo_core::resource::{Budget, Limits, Resource};
use kryo_core::task::TaskTable;
use kryo_hal::{DirectMap, UserContext};
use kryo_interfaces::{
    startup::Metadata,
    syscall::{Call, Error},
};

const LIMITS: Limits = Limits {
    shared_pages: 2,
    handles: 32,
    waits: 1,
    threads: 2,
    mappings: 32,
    tables: 24,
};

pub(crate) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    descriptor();
    wait_lifetime();
    execution(pool, direct, memory, region);
    kryo_hal::emergency_write_str("BUDGET: inheritance, retained owners and recovery OK\n");
}

fn descriptor() {
    let mut owners = ProcessTable::<(), 2, 0>::new();
    let owner = owners.insert(()).unwrap();
    let other = owners.insert(()).unwrap();
    let objects = Objects::growing();
    let account = Budget::new(LIMITS);
    let mut handles = Handles::growing();
    let object = objects
        .create(
            ObjectKind::ResourceBudget,
            KernelObject::ResourceBudget(account.clone()),
        )
        .unwrap();
    let source = handles
        .insert(object, Rights::READ | Rights::ASSIGN | Rights::TRANSFER)
        .unwrap();
    let inspect = handles
        .insert(
            handles
                .lookup(source, ObjectKind::ResourceBudget, Rights::READ)
                .unwrap(),
            Rights::READ,
        )
        .unwrap();
    let metadata = Metadata {
        service: 91,
        instance: 1,
        argument: 0,
    };
    let plan = runtime::startup::create(owner, 1, metadata, &mut handles, &objects).unwrap();
    assert_eq!(
        runtime::startup::assign_budget(owner, plan, inspect, &handles),
        Err(Error::PermissionDenied)
    );
    assert_eq!(
        runtime::startup::assign_budget(other, plan, source, &handles),
        Err(Error::PermissionDenied)
    );
    runtime::startup::assign_budget(owner, plan, source, &handles).unwrap();
    let prepared = runtime::startup::prepare(owner, plan, &handles).unwrap();
    let selected = prepared.account().unwrap();
    let charge = selected.try_charge(Resource::SharedPages, 2).unwrap();
    assert_eq!(account.count(Resource::SharedPages), 2);
    drop((prepared, charge));
    handles.close(source).unwrap();
    assert!(matches!(
        runtime::startup::prepare(owner, plan, &handles),
        Err(Error::InvalidHandle)
    ));
    assert_eq!(account.count(Resource::SharedPages), 0);
}

fn wait_lifetime() {
    let mut owners = TaskTable::<(), 2>::new();
    let owner = owners.spawn(()).unwrap();
    let other = owners.spawn(()).unwrap();
    let objects = Objects::growing();
    let account = Budget::new(LIMITS);
    let mut ipc = IpcRuntime::growing(&objects);
    let wait = ipc.begin_wait_for(owner, None, Some(&account)).unwrap();
    assert_eq!(
        ipc.begin_wait_for(other, None, Some(&account)),
        Err(Error::OutOfResources)
    );
    ipc.waits
        .complete(wait, Completion::Replied(Message::default()))
        .unwrap();
    assert_eq!(account.count(Resource::Waits), 1);
    ipc.waits.park(wait).unwrap();
    assert_eq!(account.count(Resource::Waits), 1);
    ipc.waits.take(wait).unwrap();
    assert_eq!(account.count(Resource::Waits), 0);
    let wait = ipc.begin_wait_for(other, None, Some(&account)).unwrap();
    ipc.waits.abandon(wait).unwrap();
    assert_eq!(account.count(Resource::Waits), 0);
}

fn execution(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let retained = memory.borrow().retained_pages();
    {
        let objects = Objects::growing();
        let mut tasks = Tasks::growing();
        let mut ipc = IpcRuntime::growing(&objects);
        let mut stats = CallStats::default();
        let account = Budget::new(LIMITS);
        let bytes = crate::boot::module_image("kryo-worker").unwrap();
        let mut initial = runtime::launch::prepare_budgeted(
            pool,
            direct,
            memory,
            region,
            2,
            bytes,
            Some(account.clone()),
        )
        .unwrap_or_else(|_| panic!("Budget fixture preparation"));
        let object = objects
            .create(ObjectKind::Image, KernelObject::Image(bytes))
            .unwrap();
        let image = initial
            .process
            .handles
            .insert(object, Rights::EXECUTE)
            .unwrap();
        let parent = tasks.spawn(initial).unwrap();
        assert_eq!(account.count(Resource::Threads), 1);
        let mut context: UserContext = tasks.resources_mut(parent).unwrap().context;
        context.rdi = context.rip;
        let thread =
            runtime::threads::create(parent, context, &mut tasks, &mut ipc, &mut stats, region)
                .unwrap();
        tasks
            .resources_mut(parent)
            .unwrap()
            .process
            .handles
            .close(Handle::from_raw(thread))
            .unwrap();
        assert_eq!(account.count(Resource::Threads), 2);
        assert_eq!(
            runtime::threads::create(parent, context, &mut tasks, &mut ipc, &mut stats, region),
            Err(Error::OutOfResources)
        );
        let child = tasks.ids().find(|id| *id != parent).unwrap();
        tasks.stop(child).unwrap();
        ipc.settle(&mut tasks);
        memory.borrow_mut().fail_next_restore();
        assert!(
            tasks
                .reap(child, |resources| resources.close(pool, region))
                .is_err()
        );
        assert_eq!(account.count(Resource::Threads), 2);
        drop(
            tasks
                .reap(child, |resources| resources.close(pool, region))
                .unwrap(),
        );
        assert_eq!(account.count(Resource::Threads), 1);

        context.rdi = image.as_raw();
        context.rsi = 0;
        let control =
            runtime::process::spawn(parent, context, &mut tasks, &mut ipc, &mut stats, region)
                .unwrap();
        tasks
            .resources_mut(parent)
            .unwrap()
            .process
            .handles
            .close(Handle::from_raw(control))
            .unwrap();
        assert_eq!(account.count(Resource::Threads), 2);
        assert_eq!(
            runtime::process::spawn(parent, context, &mut tasks, &mut ipc, &mut stats, region),
            Err(Error::OutOfResources)
        );
        let child = tasks.ids().find(|id| *id != parent).unwrap();
        tasks.stop(child).unwrap();
        ipc.settle(&mut tasks);
        drop(
            tasks
                .reap(child, |resources| resources.close(pool, region))
                .unwrap(),
        );
        assert_eq!(account.count(Resource::Threads), 1);

        let owner = tasks.process_for(parent).unwrap();
        let plan = runtime::startup::create(
            owner,
            1,
            Metadata {
                service: 92,
                instance: 1,
                argument: 0,
            },
            &mut tasks.resources_mut(parent).unwrap().process.handles,
            &objects,
        )
        .unwrap();
        context.rsi = plan.as_raw();
        tasks.reserve_process().unwrap();
        runtime::startup::checks::fail_publication();
        memory.borrow_mut().fail_next_restore();
        assert_eq!(
            runtime::process::spawn_manifest(
                parent, context, &mut tasks, &mut ipc, &mut stats, region
            ),
            Err(Error::OutOfResources)
        );
        assert!(
            tasks
                .resources_mut(parent)
                .unwrap()
                .process
                .pending
                .is_some()
        );
        assert_eq!(account.count(Resource::Threads), 2);
        memory.borrow_mut().fail_next_restore();
        assert_eq!(
            runtime::process::spawn_manifest(
                parent, context, &mut tasks, &mut ipc, &mut stats, region
            ),
            Err(Error::OutOfResources)
        );
        assert_eq!(account.count(Resource::Threads), 2);
        {
            let resources = tasks.resources_mut(parent).unwrap();
            assert!(
                resources
                    .process
                    .pending
                    .as_mut()
                    .unwrap()
                    .close(pool, region)
            );
            resources.process.pending = None;
            resources.process.handles.close(plan).unwrap();
        }
        assert_eq!(account.count(Resource::Threads), 1);

        context.rsi = 0;
        let control =
            runtime::process::spawn(parent, context, &mut tasks, &mut ipc, &mut stats, region)
                .unwrap();
        let child = tasks.ids().find(|id| *id != parent).unwrap();
        {
            let mut resources = tasks.resources_mut(parent).unwrap();
            resources.context.rax = Call::MemoryCreate as u64;
            resources.context.rdi = 2;
        }
        runtime::native::dispatch(parent, &mut tasks, &mut stats, &mut ipc, Some(region));
        let memory_handle = Handle::from_raw(tasks.resources_mut(parent).unwrap().context.rdx);
        assert_eq!(account.count(Resource::SharedPages), 2);
        let object = tasks
            .resources_mut(parent)
            .unwrap()
            .process
            .handles
            .lookup(memory_handle, ObjectKind::Memory, Rights::READ)
            .unwrap();
        let physical = object.with(|object| match object {
            KernelObject::Memory(memory) => memory.page_address(0).unwrap(),
            _ => unreachable!(),
        });
        tasks
            .resources_mut(child)
            .unwrap()
            .process
            .space
            .map_shared_page(
                0x60_0000,
                kryo_core::mm::vm::Permissions::ReadOnly,
                SharedLease {
                    _owner: object,
                    physical,
                },
            )
            .unwrap();
        tasks
            .resources_mut(parent)
            .unwrap()
            .process
            .handles
            .close(memory_handle)
            .unwrap();
        tasks
            .resources_mut(parent)
            .unwrap()
            .process
            .handles
            .close(Handle::from_raw(control))
            .unwrap();
        assert_eq!(account.count(Resource::SharedPages), 2);
        tasks.stop(parent).unwrap();
        ipc.settle(&mut tasks);
        drop(
            tasks
                .reap(parent, |resources| resources.close(pool, region))
                .unwrap(),
        );
        assert_eq!(account.count(Resource::Threads), 1);
        assert_eq!(account.count(Resource::SharedPages), 2);
        tasks.stop(child).unwrap();
        ipc.settle(&mut tasks);
        drop(
            tasks
                .reap(child, |resources| resources.close(pool, region))
                .unwrap(),
        );
        for resource in [
            Resource::Threads,
            Resource::Handles,
            Resource::SharedPages,
            Resource::Waits,
        ] {
            assert_eq!(account.count(resource), 0);
        }
        assert_eq!(objects.live_count(), 0);
    }
    assert_eq!(
        pool.stats().free,
        before.free - (memory.borrow().retained_pages() - retained)
    );
}
