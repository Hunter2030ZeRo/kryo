use super::shared_programs as code;
use super::{
    BootFramePool, CallStats, DirectMap, IpcRuntime, KernelMemory, KernelObject, KernelRegion,
    Objects, RefCell, Resources, Rights, TaskState, Tasks, step,
};
use kryo_core::ipc::{Endpoint, Notification};
use kryo_core::object::ObjectKind;
use kryo_hal::UserReason;

pub(super) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    run_pair(false, pool, direct, kernel, region);
    kryo_hal::emergency_write_str("SHARED: mapping and lifetime OK pages=3\n");
    run_pair(true, pool, direct, kernel, region);
    kryo_hal::emergency_write_str("TRANSFER: commit and rollback OK cases=2\n");
    check_read_only_fault(pool, direct, kernel, region);
}

fn check_read_only_fault(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let objects = Objects::new();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::with_objects(&objects);
    let mut calls = CallStats::default();
    let sender = tasks
        .spawn(Resources::new(
            pool,
            direct,
            kernel,
            region,
            2,
            &code::fault_sender(),
        ))
        .unwrap();
    let receiver = tasks
        .spawn(Resources::new(
            pool,
            direct,
            kernel,
            region,
            7,
            &code::fault_receiver(),
        ))
        .unwrap();
    let endpoint = objects
        .create(
            ObjectKind::Endpoint,
            KernelObject::Endpoint(RefCell::new(Endpoint::new(receiver))),
        )
        .unwrap();
    for (id, rights) in [(sender, Rights::SEND), (receiver, Rights::RECEIVE)] {
        let mut resources = tasks.resources_mut(id).unwrap();
        resources.context.rbp = resources
            .process
            .handles
            .insert(endpoint.try_clone().unwrap(), rights)
            .unwrap()
            .as_raw();
    }
    ipc.register(endpoint);
    let mut fault_seen = false;
    let mut reaped = 0;
    for _ in 0..64 {
        if tasks.live_count() == 0 {
            break;
        }
        assert!(tasks.has_runnable(), "Shared fault fixture deadlocked");
        let (id, outcome) = step(&mut tasks, &mut calls, &mut ipc);
        let outcome = outcome.expect("Shared fault context rejected");
        if outcome.reason == UserReason::Fault {
            assert_eq!(id, receiver);
            assert_eq!(
                (outcome.vector, outcome.error, outcome.address),
                (14, 7, 0xc0_0000)
            );
            fault_seen = true;
        }
        for id in [sender, receiver] {
            if tasks.state(id) != Ok(TaskState::Dying) {
                continue;
            }
            let resources = tasks.resources_mut(id).unwrap();
            assert_eq!(
                resources.exit_code,
                if id == sender { 41 } else { 0x100 + 14 }
            );
            assert!(resources.wait.is_none());
            assert_eq!(resources.process.handles.reserved_count(), 0);
            drop(
                tasks
                    .reap(id, |resources| resources.close(pool, region))
                    .unwrap(),
            );
            reaped += 1;
        }
    }
    assert!(fault_seen);
    assert_eq!(reaped, 2);
    assert_eq!(ipc.transfer_count(), 0);
    assert_eq!(ipc.transferred, 1);
    drop(ipc);
    assert_eq!(objects.live_count(), 0);
    assert_eq!(pool.stats(), before);
    kryo_hal::emergency_write_str("SHARED: read-only fault isolated vector=14 error=7\n");
}

fn run_pair(
    capacity_failure: bool,
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let objects = Objects::new();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::with_objects(&objects);
    let mut calls = CallStats::default();
    let producer_code = if capacity_failure {
        code::capacity_sender()
    } else {
        code::producer()
    };
    let receiver_code = if capacity_failure {
        code::capacity_receiver()
    } else {
        code::receiver()
    };
    let producer = tasks
        .spawn(Resources::new(
            pool,
            direct,
            kernel,
            region,
            2,
            &producer_code,
        ))
        .unwrap();
    let receiver = tasks
        .spawn(Resources::new(
            pool,
            direct,
            kernel,
            region,
            7,
            &receiver_code,
        ))
        .unwrap();
    let endpoint = objects
        .create(
            ObjectKind::Endpoint,
            KernelObject::Endpoint(RefCell::new(Endpoint::new(receiver))),
        )
        .unwrap();
    let mut source = tasks.resources_mut(producer).unwrap();
    source.context.rbp = source
        .process
        .handles
        .insert(endpoint.try_clone().unwrap(), Rights::SEND)
        .unwrap()
        .as_raw();
    let mut target = tasks.resources_mut(receiver).unwrap();
    target.context.rbp = target
        .process
        .handles
        .insert(
            endpoint.try_clone().unwrap(),
            Rights::SEND | Rights::RECEIVE | Rights::CONTROL,
        )
        .unwrap()
        .as_raw();
    ipc.register(endpoint);
    if !capacity_failure {
        let notification = objects
            .create(
                ObjectKind::Notification,
                KernelObject::Notification(RefCell::new(Notification::new(receiver))),
            )
            .unwrap();
        let mut source = tasks.resources_mut(producer).unwrap();
        source.context.rbx = source
            .process
            .handles
            .insert(notification.try_clone().unwrap(), Rights::SIGNAL)
            .unwrap()
            .as_raw();
        let mut target = tasks.resources_mut(receiver).unwrap();
        target.context.rbx = target
            .process
            .handles
            .insert(notification.try_clone().unwrap(), Rights::WAIT)
            .unwrap()
            .as_raw();
        ipc.register(notification);
    }
    let mut reaped = [false; 2];
    for _ in 0..256 {
        if tasks.live_count() == 0 {
            break;
        }
        assert!(tasks.has_runnable(), "Shared-memory fixture deadlocked");
        let (_, outcome) = step(&mut tasks, &mut calls, &mut ipc);
        assert_ne!(
            outcome.expect("Shared task context rejected").reason,
            UserReason::Fault
        );
        for (index, id) in [producer, receiver].into_iter().enumerate() {
            if tasks.state(id) != Ok(TaskState::Dying) {
                continue;
            }
            let resources = tasks.resources_mut(id).unwrap();
            let expected = if capacity_failure { [33, 34] } else { [31, 32] };
            assert_eq!(resources.exit_code, expected[index]);
            assert!(resources.wait.is_none() && resources.receive_slots.is_none());
            assert_eq!(resources.process.handles.reserved_count(), 0);
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
    assert_eq!(ipc.transfer_count(), 0);
    assert_eq!(ipc.waits.next_ready(), None);
    assert_eq!(ipc.transferred, if capacity_failure { 0 } else { 2 });
    assert_eq!(ipc.out_of_order, usize::from(!capacity_failure));
    assert_eq!(calls.denied, if capacity_failure { 0 } else { 2 });
    drop(ipc);
    assert_eq!(objects.live_count(), 0);
    assert_eq!(pool.stats(), before);
}
