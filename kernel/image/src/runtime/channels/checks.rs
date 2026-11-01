use super::*;
use crate::runtime::{Objects, Tasks};
use kryo_core::ipc::ChannelBudget;
use kryo_core::task::TaskTable;

pub(crate) fn run() {
    let mut owners = TaskTable::<(), 1>::new();
    let owner = owners.spawn(()).unwrap();
    handle_failure(owner);
    arena_failure(owner);
    registry_failure(owner);
    quota_retirement(owner);
    owners.stop(owner).unwrap();
    owners.reap(owner, |_| Ok::<_, ()>(())).unwrap();
    kryo_hal::emergency_write_str("CHANNEL: creation rollback and lifetime OK\n");
}

fn factory<'objects>(
    objects: &'objects Objects,
    handles: &mut Handles<'objects>,
    budget: &ChannelBudget,
) -> Handle {
    let object = objects
        .create(
            ObjectKind::ChannelFactory,
            KernelObject::ChannelFactory(budget.clone()),
        )
        .unwrap();
    handles
        .insert(object, Rights::CREATE | Rights::DUP)
        .unwrap()
}

fn handle_failure(owner: TaskId) {
    let objects = Objects::growing();
    let mut ipc = IpcRuntime::growing(&objects);
    let budget = ChannelBudget::new(1);
    let mut handles = Handles::limited(1);
    let factory = factory(&objects, &mut handles, &budget);
    assert_eq!(
        create(owner, factory, Call::EndpointCreate, &mut handles, &mut ipc),
        Err(Error::OutOfResources)
    );
    assert_eq!(handles.reserved_count(), 0);
    assert_eq!(budget.live_count(), 0);
    assert_eq!(objects.live_count(), 1);
}

fn arena_failure(owner: TaskId) {
    let objects = Objects::limited(1);
    let mut ipc = IpcRuntime::growing(&objects);
    let budget = ChannelBudget::new(1);
    let mut handles = Handles::growing();
    let factory = factory(&objects, &mut handles, &budget);
    assert_eq!(
        create(
            owner,
            factory,
            Call::NotificationCreate,
            &mut handles,
            &mut ipc
        ),
        Err(Error::OutOfResources)
    );
    assert_eq!(handles.reserved_count(), 0);
    assert_eq!(budget.live_count(), 0);
    assert_eq!(objects.live_count(), 1);
}

fn registry_failure(owner: TaskId) {
    let objects = Objects::growing();
    let mut ipc = IpcRuntime::with_objects(&objects);
    let budget = ChannelBudget::new(1);
    let mut handles = Handles::growing();
    let factory = factory(&objects, &mut handles, &budget);
    for _ in 0..4 {
        let channel = objects
            .create(
                ObjectKind::Notification,
                KernelObject::Notification(RefCell::new(Notification::new(owner))),
            )
            .unwrap();
        ipc.try_register(channel).unwrap();
    }
    assert_eq!(
        create(owner, factory, Call::EndpointCreate, &mut handles, &mut ipc),
        Err(Error::OutOfResources)
    );
    assert_eq!(handles.reserved_count(), 0);
    assert_eq!(budget.live_count(), 0);
    assert_eq!(objects.live_count(), 5);
}

fn quota_retirement(owner: TaskId) {
    let objects = Objects::growing();
    let mut ipc = IpcRuntime::growing(&objects);
    let mut tasks = Tasks::growing();
    let budget = ChannelBudget::new(1);
    let mut handles = Handles::growing();
    let factory = factory(&objects, &mut handles, &budget);
    assert_eq!(
        create(
            owner,
            Handle::INVALID,
            Call::EndpointCreate,
            &mut handles,
            &mut ipc
        ),
        Err(Error::InvalidHandle)
    );
    let wrong = objects
        .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
        .unwrap();
    let wrong = handles.insert(wrong, Rights::CREATE).unwrap();
    assert_eq!(
        create(
            owner,
            wrong,
            Call::NotificationCreate,
            &mut handles,
            &mut ipc
        ),
        Err(Error::WrongType)
    );
    handles.close(wrong).unwrap();
    let reduced = handles.duplicate(factory, Rights::DUP).unwrap();
    assert_eq!(
        create(owner, reduced, Call::EndpointCreate, &mut handles, &mut ipc),
        Err(Error::PermissionDenied)
    );
    let first = create(owner, factory, Call::EndpointCreate, &mut handles, &mut ipc).unwrap();
    assert_eq!(
        create(
            owner,
            factory,
            Call::NotificationCreate,
            &mut handles,
            &mut ipc
        ),
        Err(Error::OutOfResources)
    );
    let channel = handles
        .lookup(first, ObjectKind::Endpoint, Rights::CONTROL)
        .unwrap();
    let first_id = channel.id();
    channel.with(|object| {
        let KernelObject::Endpoint(endpoint) = object else {
            unreachable!()
        };
        endpoint.borrow_mut().close(&mut ipc.waits);
    });
    drop(channel);
    ipc.settle(&mut tasks);
    assert_eq!(
        budget.live_count(),
        1,
        "Closed external reference must retain its charge"
    );
    handles.close(first).unwrap();
    ipc.settle(&mut tasks);
    assert_eq!(budget.live_count(), 0);
    let second = create(
        owner,
        factory,
        Call::NotificationCreate,
        &mut handles,
        &mut ipc,
    )
    .unwrap();
    let channel = handles
        .lookup(second, ObjectKind::Notification, Rights::WAIT)
        .unwrap();
    assert_ne!(channel.id(), first_id);
    drop(channel);
    handles.close(second).unwrap();
    ipc.settle(&mut tasks);
    assert_eq!(
        budget.live_count(),
        0,
        "Registry-only notification must retire"
    );
    assert_eq!(handles.reserved_count(), 0);
    assert_eq!(objects.live_count(), 1);
    handles.close(factory).unwrap();
    handles.close(reduced).unwrap();
    assert_eq!(objects.live_count(), 0);
}
