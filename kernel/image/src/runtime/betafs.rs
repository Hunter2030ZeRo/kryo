use super::{
    BootFramePool, CallStats, IpcRuntime, KernelMemory, KernelObject, KernelRegion, Objects,
    RefCell, Tasks,
};
use kryo_core::capability::Rights;
use kryo_core::ipc::{ChannelBudget, Endpoint};
use kryo_core::object::ObjectKind;
use kryo_core::process::{ExitReason, ExitStatus};
use kryo_core::resource::{Budget, Limits, Resource};
use kryo_core::sched::TaskState;
use kryo_core::task::TaskId;
use kryo_hal::DirectMap;
use kryo_interfaces::startup::{Entry, Metadata, ObjectKind as WireKind};
use kryo_interfaces::syscall::Call;

pub(crate) fn phase() -> Option<bool> {
    match crate::boot::module_image("kryo-betafs-probe") {
        Ok(b"KRYO-BETAFS-FORMAT-V1\n") => Some(true),
        Ok(b"KRYO-BETAFS-OPEN-V1\n") => Some(false),
        Ok(_) => panic!("Invalid BetAFS probe configuration"),
        Err(crate::boot::ModuleError::Missing) => None,
        Err(error) => panic!("Invalid BetAFS module: {:?}", error),
    }
}

pub(crate) fn check(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let Some(format) = phase() else { return };
    check_codecs(pool, direct, memory, region);
    check_volume(pool, direct, memory, region, format);
}

fn native_budget() -> Budget {
    Budget::new(Limits {
        shared_pages: 64,
        private_pages: 32768,
        reserved_pages: 131072,
        handles: 256,
        waits: 16,
        threads: 8,
        mappings: 65536,
        tables: 1024,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "Trusted startup installs a native image before publication"
)]
fn prepare<'kernel, 'objects>(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    tasks: &mut Tasks<'kernel, 'objects>,
    name: &str,
    argv: &[&[u8]],
    budget: &Budget,
) -> TaskId {
    let bytes =
        crate::boot::module_image_limited(name, 8 << 20).expect("Missing BetAFS native image");
    let slot = region
        .find_stack(pool, direct)
        .expect("No BetAFS entry stack");
    let resources = super::launch::prepare_native_with_stack(
        pool,
        direct,
        memory,
        region,
        slot,
        super::launch::NativeImage {
            bytes,
            argv,
            envp: &[],
        },
        Some(budget.clone()),
        256,
    )
    .unwrap_or_else(|mut failure| {
        failure.pending.close(pool, region);
        panic!("Cannot load BetAFS native program: {:?}", failure.error);
    });
    tasks
        .spawn_suspended(resources)
        .unwrap_or_else(|_| panic!("Cannot publish BetAFS process"))
}

fn check_codecs(
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
        let budget = native_budget();
        let task = prepare(
            pool,
            direct,
            memory,
            region,
            &mut tasks,
            "kryo-betafs-codecs",
            &[b"betafs-codecs"],
            &budget,
        );
        let owner = tasks.process_for(task).unwrap();
        let debug = objects
            .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
            .unwrap();
        super::block_service::install(
            task,
            Metadata {
                service: 61,
                instance: 1,
                argument: 0,
            },
            &[(1, &debug, Rights::WRITE | Rights::DUP)],
            &mut tasks,
            &objects,
        );
        tasks.wake(task).unwrap();
        let mut calls = CallStats::default();
        let mut complete = false;
        let mut exited = false;
        let deadline = now() + 120_000_000_000;
        loop {
            tick(&mut tasks, &mut ipc, &mut calls, region, deadline);
            if let Some((source, event, value)) = calls.last_debug.take() {
                assert_eq!(source, task);
                assert_ne!(event, 399, "Native codec failed at stage {}", value);
                assert_eq!((event, value), (300, 15));
                assert!(!complete, "Duplicate codec completion");
                complete = true;
            }
            if let Some(status) = tasks.terminal(owner) {
                assert_eq!(
                    status,
                    ExitStatus {
                        reason: ExitReason::Exited,
                        code: 0
                    }
                );
                exited = true;
            }
            reap(&mut tasks, &mut ipc, pool, region);
            if exited && tasks.live_count() == 0 {
                break;
            }
        }
        assert!(complete);
        assert_eq!(calls.threads_created, 0);
        assert_eq!(ipc.waits.live_count(), 0);
        drop(ipc);
        drop(debug);
        assert_eq!(objects.live_count(), 0);
        check_budget(&budget);
    }
    assert_eq!(
        pool.stats().free + memory.borrow().retained_pages(),
        before.free + retained
    );
    kryo_hal::emergency_write_str("BETAFS: native Raw Zstd LZ4 OpenZL and engine checks OK\n");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Mount,
    Client,
    Holder,
    Waiter,
    Recovery,
    Unmount,
    Stop,
}

fn check_volume(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
    format: bool,
) {
    let before = pool.stats();
    let retained = memory.borrow().retained_pages();
    {
        let objects = Objects::growing();
        let mut tasks = Tasks::growing();
        let mut ipc = IpcRuntime::growing(&objects);
        let driver_budget = super::block_service::budget();
        let driver = super::block_service::prepare_driver(
            pool,
            direct,
            memory,
            region,
            &objects,
            &mut tasks,
            &mut ipc,
            &driver_budget,
            true,
        );
        let budgets: [Budget; 4] = core::array::from_fn(|_| native_budget());
        let fs = prepare(
            pool,
            direct,
            memory,
            region,
            &mut tasks,
            "kryo-betafs-service",
            &[b"betafs-service"],
            &budgets[0],
        );
        let fs_owner = tasks.process_for(fs).unwrap();
        let channels = ChannelBudget::new(256);
        let endpoint = objects
            .create(
                ObjectKind::Endpoint,
                KernelObject::Endpoint(RefCell::new(Endpoint::leased(
                    fs,
                    channels.try_reserve().unwrap(),
                ))),
            )
            .unwrap();
        ipc.try_register(endpoint.try_clone().unwrap()).unwrap();
        super::block_service::install(
            fs,
            Metadata {
                service: 60,
                instance: 1,
                argument: 2 | u64::from(format),
            },
            &[
                (1, &driver.debug, Rights::WRITE | Rights::DUP),
                (10, &endpoint, Rights::RECEIVE | Rights::CONTROL),
                (20, &driver.gates[0], Rights::SEND),
                (21, &driver.gates[1], Rights::SEND),
            ],
            &mut tasks,
            &objects,
        );
        let mut namespace = Some({
            let resources = tasks.resources_mut(fs).unwrap();
            let gate = ipc
                .create_gate(
                    fs,
                    endpoint.try_clone().unwrap(),
                    kryo_interfaces::file::NAMESPACE_LABEL,
                    &mut resources.process.handles,
                )
                .unwrap();
            let object = resources
                .process
                .handles
                .lookup(gate, ObjectKind::EndpointGate, Rights::SEND)
                .unwrap();
            resources.process.handles.close(gate).unwrap();
            object
        });
        let names: [&[u8]; 3] = [
            if format {
                b"betafs-write"
            } else {
                b"betafs-verify"
            },
            b"betafs-lock-owner",
            b"betafs-lock-waiter",
        ];
        let clients: [TaskId; 3] = core::array::from_fn(|index| {
            let task = prepare(
                pool,
                direct,
                memory,
                region,
                &mut tasks,
                "kryo-betafs-client",
                &[b"rust-std-probe", names[index]],
                &budgets[index + 1],
            );
            install_client(
                task,
                u64::from(index as u32) + 62,
                namespace.as_ref().unwrap(),
                &driver.debug,
                &mut tasks,
                &objects,
            );
            task
        });
        let owners = clients.map(|task| tasks.process_for(task).unwrap());
        let stopper_budget = super::block_service::budget();
        let stopper = super::block_service::prepare(
            pool,
            direct,
            memory,
            region,
            &mut tasks,
            "kryo-block-client",
            &stopper_budget,
        );
        let stopper_owner = tasks.process_for(stopper).unwrap();
        super::block_service::install(
            stopper,
            Metadata {
                service: 44,
                instance: 1,
                argument: 2,
            },
            &[
                (
                    kryo_interfaces::block::role::CONTROL_GATE,
                    &driver.gates[2],
                    Rights::SEND,
                ),
                (
                    kryo_interfaces::block::role::DEBUG,
                    &driver.debug,
                    Rights::WRITE,
                ),
            ],
            &mut tasks,
            &objects,
        );
        tasks.wake(driver.task).unwrap();
        tasks.wake(fs).unwrap();
        let mut phase = Phase::Mount;
        let mut calls = CallStats::default();
        let mut client_done = [false; 3];
        let mut main_event = false;
        let mut holder_event = false;
        let mut waiter_event = false;
        let mut recovered = false;
        let mut fs_done = false;
        let mut driver_done = false;
        let mut stopper_done = false;
        let mut syncs = 0;
        let mut sync_started = None;
        let deadline = now() + 180_000_000_000;
        loop {
            assert!(
                now() < deadline,
                "BetAFS stalled phase={:?} clients={:?} fs={} driver={} stopper={}",
                phase,
                client_done,
                fs_done,
                driver_done,
                stopper_done
            );
            tick(&mut tasks, &mut ipc, &mut calls, region, deadline);
            if let Some((source, event, value)) = calls.last_debug.take() {
                assert_ne!(
                    event,
                    399,
                    "BetAFS failed at stage {} source={:#x}",
                    value,
                    source.as_raw()
                );
                assert_ne!(event, 229, "Persistent std probe failed at stage {}", value);
                match event {
                    310 => {
                        assert_eq!(source, fs);
                        assert_eq!(value, u64::from(format));
                        assert!(phase == Phase::Mount);
                        tasks.wake(clients[0]).unwrap();
                        phase = Phase::Client;
                    }
                    311 => {
                        assert_eq!(source, fs);
                        assert_eq!(value, syncs + 1);
                        let before = sync_started
                            .take()
                            .expect("Barrier completed without start");
                        assert!(
                            ipc.transport.as_ref().unwrap().completed_flushes() > before,
                            "File sync succeeded without a completed VirtIO FLUSH"
                        );
                        syncs = value;
                    }
                    312 => {
                        assert_eq!(source, fs);
                        assert_eq!(value, syncs + 1);
                        assert!(sync_started.is_none());
                        sync_started = Some(ipc.transport.as_ref().unwrap().completed_flushes());
                    }
                    320 | 321 => {
                        assert_eq!(source, clients[0]);
                        assert_eq!(event, if format { 320 } else { 321 });
                        assert_eq!(value, 1);
                        assert!(!main_event);
                        main_event = true;
                    }
                    324 => {
                        assert_eq!(source, clients[1]);
                        assert_eq!(value, 1);
                        holder_event = true;
                    }
                    325 => {
                        assert_eq!(source, clients[2]);
                        assert_eq!(value, 1);
                        waiter_event = true;
                    }
                    326 => {
                        assert_eq!(source, clients[2]);
                        assert_eq!(value, 1);
                        recovered = true;
                    }
                    _ => {}
                }
            }
            for index in 0..3 {
                if let Some(status) = tasks.terminal(owners[index]) {
                    assert_eq!(
                        status.reason,
                        if index == 1 {
                            ExitReason::Stopped
                        } else {
                            ExitReason::Exited
                        }
                    );
                    assert_eq!(
                        status.code,
                        if index == 1 { 91 } else { 0 },
                        "BetAFS client failed index={}",
                        index
                    );
                    client_done[index] = true;
                }
            }
            if let Some(status) = tasks.terminal(fs_owner) {
                assert_eq!(
                    status,
                    ExitStatus {
                        reason: ExitReason::Exited,
                        code: 0
                    },
                    "BetAFS service failed"
                );
                fs_done = true;
            }
            if let Some(status) = tasks.terminal(driver.owner) {
                assert_eq!(
                    status,
                    ExitStatus {
                        reason: ExitReason::Exited,
                        code: 0
                    },
                    "BetAFS driver failed"
                );
                driver_done = true;
            }
            if let Some(status) = tasks.terminal(stopper_owner) {
                assert_eq!(
                    status,
                    ExitStatus {
                        reason: ExitReason::Exited,
                        code: 0
                    }
                );
                stopper_done = true;
            }
            match phase {
                Phase::Client if client_done[0] => {
                    assert!(main_event && syncs >= 3);
                    tasks.wake(clients[1]).unwrap();
                    phase = Phase::Holder;
                }
                Phase::Holder if holder_event => {
                    tasks.wake(clients[2]).unwrap();
                    phase = Phase::Waiter;
                }
                Phase::Waiter if waiter_event && budgets[3].count(Resource::Waits) == 1 => {
                    assert_eq!(tasks.state(clients[2]), Ok(TaskState::Blocked));
                    assert_eq!(
                        tasks.thread(clients[2]).unwrap().context.rax,
                        Call::CallExchange as u64
                    );
                    tasks
                        .stop_process(
                            owners[1],
                            ExitStatus {
                                reason: ExitReason::Stopped,
                                code: 91,
                            },
                        )
                        .unwrap();
                    tasks.end_thread(clients[1]).unwrap();
                    assert_eq!(
                        tasks.terminal(owners[1]),
                        Some(ExitStatus {
                            reason: ExitReason::Stopped,
                            code: 91,
                        })
                    );
                    client_done[1] = true;
                    phase = Phase::Recovery;
                }
                Phase::Recovery if client_done[1] && client_done[2] => {
                    assert!(recovered);
                    drop(namespace.take());
                    ipc.settle(&mut tasks);
                    phase = Phase::Unmount;
                }
                Phase::Unmount if fs_done => {
                    tasks.wake(stopper).unwrap();
                    phase = Phase::Stop;
                }
                _ => {}
            }
            reap(&mut tasks, &mut ipc, pool, region);
            if phase == Phase::Stop
                && fs_done
                && driver_done
                && stopper_done
                && tasks.live_count() == 0
            {
                break;
            }
        }
        assert!(main_event && holder_event && waiter_event && recovered);
        assert_eq!(ipc.waits.live_count(), 0);
        assert!(ipc.words.is_empty());
        assert!(ipc.transport.is_none());
        drop(ipc);
        let driver_channels = driver.channels.clone();
        drop(driver);
        drop(endpoint);
        assert_eq!(objects.live_count(), 0);
        assert_eq!(channels.live_count(), 0);
        assert_eq!(driver_channels.live_count(), 0);
        for budget in &budgets {
            check_budget(budget);
        }
        check_budget(&driver_budget);
        check_budget(&stopper_budget);
    }
    assert_eq!(
        pool.stats().free + memory.borrow().retained_pages(),
        before.free + retained
    );
    kryo_hal::emergency_write_str(if format {
        "BETAFS: file sync_data sync_all and directory barriers OK phase=write\n"
    } else {
        "BETAFS: reopened data and metadata OK phase=verify\n"
    });
    kryo_hal::emergency_write_str("BETAFS: blocked lock owner death recovery OK\n");
    kryo_hal::emergency_write_str("BETAFS: service driver and backing retirement OK\n");
}

fn install_client<'objects>(
    task: TaskId,
    service: u64,
    namespace: &super::Object<'objects>,
    debug: &super::Object<'objects>,
    tasks: &mut Tasks<'_, 'objects>,
    objects: &'objects Objects,
) {
    let mut resources = tasks.resources_mut(task).unwrap();
    let mut entries = alloc::vec::Vec::new();
    for (role, object, kind, rights) in [
        (
            1,
            debug,
            WireKind::DebugConsole,
            Rights::WRITE | Rights::DUP,
        ),
        (2, namespace, WireKind::EndpointGate, Rights::SEND),
    ] {
        let handle = resources
            .process
            .handles
            .insert(object.try_clone().unwrap(), rights)
            .unwrap();
        entries.push(Entry {
            role,
            kind,
            rights: rights.bits(),
            handle: handle.as_raw(),
        });
    }
    let record = super::startup::install_record(
        Metadata {
            service,
            instance: 1,
            argument: 0,
        },
        &entries,
        &mut resources.process.handles,
        objects,
    )
    .unwrap();
    resources.context.rdi = record.as_raw();
    resources.context.rsi = 0;
}

fn tick<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    calls: &mut CallStats,
    region: &mut KernelRegion,
    deadline: u64,
) {
    assert!(now() < deadline, "BetAFS native graph stalled");
    super::expire_waits(tasks, ipc);
    if tasks.has_runnable() {
        super::smp::step(tasks, calls, ipc, region);
    } else if ipc.waits.next_deadline().is_some() {
        super::idle_until_deadline(tasks, ipc);
    } else {
        unsafe { kryo_hal::arm_deadline(deadline.min(now() + 1_000_000)) }.unwrap();
        kryo_hal::idle_until_interrupt().unwrap();
        unsafe { kryo_hal::stop_timer() }.unwrap();
    }
}

fn reap<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    pool: &BootFramePool,
    region: &mut KernelRegion,
) {
    ipc.settle(tasks);
    if super::devices::retire_bindings(tasks, ipc).is_err() {
        return;
    }
    let mut cursor = None;
    while let Some(id) = tasks.next_dying(cursor) {
        cursor = Some(id);
        if let Ok(resources) = tasks.reap(id, ipc, |resources| resources.close(pool, region)) {
            drop(resources);
        }
    }
}

fn check_budget(budget: &Budget) {
    for resource in [
        Resource::SharedPages,
        Resource::PrivatePages,
        Resource::ReservedPages,
        Resource::Handles,
        Resource::Waits,
        Resource::Threads,
    ] {
        assert_eq!(
            budget.count(resource),
            0,
            "BetAFS resource survived: {:?}",
            resource
        );
    }
}

fn now() -> u64 {
    kryo_hal::monotonic_time().expect("No BetAFS monotonic clock")
}
