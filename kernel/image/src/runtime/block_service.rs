use super::{
    BootFramePool, IpcRuntime, KernelMemory, KernelObject, KernelRegion, Object, Objects, RefCell,
    Tasks,
};
use kryo_core::capability::Rights;
use kryo_core::ipc::{ChannelBudget, Delivery, Endpoint};
use kryo_core::object::ObjectKind;
use kryo_core::process::{ExitReason, ProcessId};
use kryo_core::resource::{Budget, Limits};
use kryo_core::sched::TaskState;
use kryo_core::task::TaskId;
use kryo_hal::{DirectMap, VirtioPci};
use kryo_interfaces::block::role;
use kryo_interfaces::startup::{Entry, Metadata, ObjectKind as WireKind};
use kryo_interfaces::syscall::Call;
#[cfg(feature = "boot-tests")]
use {super::CallStats, kryo_core::resource::Resource};

pub(crate) struct Graph<'objects> {
    driver: TaskId,
    driver_process: ProcessId,
    clients: [TaskId; 3],
    processes: [ProcessId; 3],
    complete: [bool; 3],
    phase: u8,
    gates: [Object<'objects>; 3],
    debug: Object<'objects>,
    accounts: [Budget; 5],
    _channels: ChannelBudget,
    stopper: Option<TaskId>,
    test: bool,
}

pub(crate) fn enabled() -> bool {
    crate::boot::module_image("kryo-block-probe-enabled").ok()
        == Some(b"KRYO-BLOCK-PROBE-V1\n".as_slice())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Trusted boot connects owned runtime components"
)]
pub(crate) fn start<'kernel, 'objects>(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    objects: &'objects Objects,
    tasks: &mut Tasks<'kernel, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    test: bool,
) -> Option<Graph<'objects>> {
    if !enabled() {
        return None;
    }
    let function = crate::boot::storage::block_device().expect("Missing service block device");
    let mut hardware = unsafe { VirtioPci::new(function) }.expect("Cannot claim service device");
    let deadline = now() + 2_000_000_000;
    while !hardware.reset().expect("Cannot reset service device") {
        pause(deadline);
    }
    hardware
        .prepare()
        .expect("Cannot acknowledge service device");
    let accounts = core::array::from_fn(|_| budget());
    let driver = prepare(
        pool,
        direct,
        memory,
        region,
        tasks,
        "kryo-block-service",
        &accounts[0],
    );
    assert_eq!(tasks.state(driver), Ok(TaskState::Blocked));
    let owner = tasks.process_for(driver).unwrap();
    let device = objects
        .create(ObjectKind::Device, KernelObject::Transport(owner))
        .unwrap();
    ipc.transport = Some(
        super::transport::Transport::create(
            owner,
            device.try_clone().unwrap(),
            hardware,
            pool,
            direct,
            memory,
        )
        .unwrap_or_else(|_| panic!("Cannot create service DMA storage")),
    );
    let channels = ChannelBudget::new(3);
    let gates = core::array::from_fn(|_| {
        let object = objects
            .create(
                ObjectKind::Endpoint,
                KernelObject::Endpoint(RefCell::new(Endpoint::leased(
                    driver,
                    channels.try_reserve().unwrap(),
                ))),
            )
            .unwrap();
        ipc.try_register(object.try_clone().unwrap())
            .unwrap_or_else(|_| panic!("Cannot register block gate"));
        object
    });
    let debug = objects
        .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
        .unwrap();
    install(
        driver,
        Metadata {
            service: 40,
            instance: 1,
            argument: u64::from(test),
        },
        &[
            (
                role::DEVICE,
                &device,
                Rights::READ | Rights::WRITE | Rights::CONTROL,
            ),
            (role::READ_GATE, &gates[0], Rights::RECEIVE),
            (role::WRITE_GATE, &gates[1], Rights::RECEIVE),
            (role::CONTROL_GATE, &gates[2], Rights::RECEIVE),
            (role::DEBUG, &debug, Rights::WRITE),
        ],
        tasks,
        objects,
    );
    let clients = core::array::from_fn(|index| {
        let mode = [3, 0, 1][index];
        let client = prepare(
            pool,
            direct,
            memory,
            region,
            tasks,
            "kryo-block-client",
            &accounts[index + 1],
        );
        let mut grants = alloc::vec![
            (role::READ_GATE, &gates[0], Rights::SEND),
            (role::DEBUG, &debug, Rights::WRITE)
        ];
        if mode == 1 {
            grants.push((role::WRITE_GATE, &gates[1], Rights::SEND));
        }
        install(
            client,
            Metadata {
                service: 41 + index as u64,
                instance: 1,
                argument: mode,
            },
            &grants,
            tasks,
            objects,
        );
        client
    });
    let processes = clients.map(|id| tasks.process_for(id).unwrap());
    // Every capability and immutable receipt is installed before Runnable.
    tasks.wake(driver).unwrap();
    let _ = kryo_hal::print(format_args!(
        "BLOCK IPC: service dispatched id={:#x}\n",
        driver.as_raw()
    ));
    Some(Graph {
        driver,
        driver_process: owner,
        clients,
        processes,
        complete: [false; 3],
        phase: 0,
        gates,
        debug,
        accounts,
        _channels: channels,
        stopper: None,
        test,
    })
}

impl<'objects> Graph<'objects> {
    #[expect(
        clippy::too_many_arguments,
        reason = "Deferred trusted client startup uses runtime owners"
    )]
    pub(crate) fn observe<'kernel>(
        &mut self,
        pool: &'static BootFramePool,
        direct: DirectMap,
        memory: &'kernel RefCell<KernelMemory>,
        region: &mut KernelRegion,
        objects: &'objects Objects,
        tasks: &mut Tasks<'kernel, 'objects>,
        ipc: &mut IpcRuntime<'objects>,
    ) {
        if !self.test
            && let Some(status) = tasks.terminal(self.driver_process)
        {
            if self.phase != u8::MAX {
                let _ = kryo_hal::print(format_args!(
                    "BLOCK IPC: service offline status={:?}\n",
                    status
                ));
                self.phase = u8::MAX;
            }
            return;
        }
        for index in 0..3 {
            if let Some(status) = tasks.terminal(self.processes[index]) {
                assert_eq!(status.reason, ExitReason::Exited);
                assert_eq!(status.code, 0, "Block IPC client failed index={}", index);
                self.complete[index] = true;
            }
        }
        if self.phase == 0 && receiving(tasks, self.driver) {
            tasks.wake(self.clients[0]).unwrap();
            self.phase = 1;
        }
        if self.phase == 1
            && ipc
                .transport
                .as_ref()
                .is_some_and(|transport| transport.has_pending())
            && let Some(wait) = tasks.thread(self.clients[0]).unwrap().wait
        {
            assert_eq!(ipc.cancel(wait).unwrap(), Delivery::Accepted);
            ipc.settle(tasks);
            self.phase = 2;
            kryo_hal::emergency_write_str("BLOCK IPC: accepted cancellation retains loan\n");
        }
        if self.phase == 2 && self.complete[0] {
            tasks.wake(self.clients[1]).unwrap();
            tasks.wake(self.clients[2]).unwrap();
            self.phase = 3;
        }
        if self.phase == 3
            && self.complete.iter().all(|done| *done)
            && receiving(tasks, self.driver)
        {
            assert!(
                ipc.transport
                    .as_ref()
                    .is_some_and(|transport| !transport.has_pending())
            );
            self.phase = 4;
            kryo_hal::emergency_write_str("BLOCK IPC: clients complete; service waiting\n");
            if self.test {
                let task = prepare(
                    pool,
                    direct,
                    memory,
                    region,
                    tasks,
                    "kryo-block-client",
                    &self.accounts[4],
                );
                install(
                    task,
                    Metadata {
                        service: 44,
                        instance: 1,
                        argument: 2,
                    },
                    &[
                        (role::CONTROL_GATE, &self.gates[2], Rights::SEND),
                        (role::DEBUG, &self.debug, Rights::WRITE),
                    ],
                    tasks,
                    objects,
                );
                tasks.wake(task).unwrap();
                self.stopper = Some(task);
            }
        }
    }
}

#[cfg(feature = "boot-tests")]
pub(crate) fn check(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    if !enabled() {
        return;
    }
    let before = pool.stats();
    let retained = memory.borrow().retained_pages();
    {
        let objects = Objects::growing();
        let mut tasks = Tasks::growing();
        let mut ipc = IpcRuntime::growing(&objects);
        let mut graph = start(
            pool, direct, memory, region, &objects, &mut tasks, &mut ipc, true,
        )
        .unwrap();
        let mut calls = CallStats::default();
        let deadline = now() + 10_000_000_000;
        loop {
            assert!(now() < deadline, "Block IPC graph stalled");
            super::expire_waits(&mut tasks, &mut ipc);
            graph.observe(pool, direct, memory, region, &objects, &mut tasks, &mut ipc);
            if let Some(status) = tasks.terminal(graph.driver_process) {
                assert_eq!(status.code, 0);
                if graph
                    .stopper
                    .is_some_and(|task| tasks.state(task) == Ok(TaskState::Dying))
                {
                    break;
                }
            }
            if tasks.has_runnable() {
                super::smp::step(&mut tasks, &mut calls, &mut ipc, region);
            } else if ipc.waits.next_deadline().is_some() {
                super::idle_until_deadline(&mut tasks, &mut ipc);
            } else {
                pause(deadline);
            }
        }
        while super::devices::retire_bindings(&tasks, &mut ipc).is_err() {
            pause(deadline);
        }
        ipc.settle(&mut tasks);
        let stopper = graph.stopper.unwrap();
        for task in [
            graph.driver,
            graph.clients[0],
            graph.clients[1],
            graph.clients[2],
            stopper,
        ] {
            assert_eq!(tasks.state(task), Ok(TaskState::Dying));
            drop(
                tasks
                    .reap(task, |resources| resources.close(pool, region))
                    .unwrap(),
            );
        }
        drop(ipc);
        let accounts = graph.accounts.clone();
        let channels = graph._channels.clone();
        drop(graph);
        assert_eq!(objects.live_count(), 0);
        assert_eq!(channels.live_count(), 0);
        for account in accounts {
            for resource in [
                Resource::SharedPages,
                Resource::Handles,
                Resource::Waits,
                Resource::Threads,
            ] {
                assert_eq!(account.count(resource), 0);
            }
        }
    }
    assert_eq!(
        pool.stats().free + memory.borrow().retained_pages() - retained,
        before.free
    );
    kryo_hal::emergency_write_str("BLOCK IPC: graceful stop and resource retirement OK\n");
}

fn receiving(tasks: &Tasks<'_, '_>, task: TaskId) -> bool {
    tasks.state(task) == Ok(TaskState::Blocked)
        && tasks.thread(task).is_ok_and(|thread| {
            thread.wait.is_some() && thread.context.rax == Call::ReceiveFor as u64
        })
}

fn budget() -> Budget {
    Budget::new(Limits {
        shared_pages: 8,
        handles: 64,
        waits: 8,
        threads: 1,
        mappings: 64,
        tables: 32,
    })
}

fn prepare<'kernel, 'objects>(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    tasks: &mut Tasks<'kernel, 'objects>,
    image: &str,
    account: &Budget,
) -> TaskId {
    let bytes = crate::boot::module_image(image).expect("Missing block service program");
    let slot = region
        .find_stack(pool, direct)
        .expect("No block entry-stack cell");
    let resources = super::launch::prepare_budgeted(
        pool,
        direct,
        memory,
        region,
        slot,
        bytes,
        Some(account.clone()),
    )
    .unwrap_or_else(|_| panic!("Cannot load block service program"));
    tasks
        .spawn_suspended(resources)
        .unwrap_or_else(|_| panic!("Cannot prepare block process"))
}

fn install<'objects>(
    task: TaskId,
    metadata: Metadata,
    grants: &[(u64, &Object<'objects>, Rights)],
    tasks: &mut Tasks<'_, 'objects>,
    objects: &'objects Objects,
) {
    let mut resources = tasks.resources_mut(task).unwrap();
    let mut entries = alloc::vec::Vec::new();
    for (role, object, rights) in grants {
        let kind = match object.kind() {
            ObjectKind::Device => WireKind::Device,
            ObjectKind::Endpoint => WireKind::Endpoint,
            ObjectKind::DebugConsole => WireKind::DebugConsole,
            _ => unreachable!(),
        };
        let handle = resources
            .process
            .handles
            .insert(object.try_clone().unwrap(), *rights)
            .unwrap();
        entries.push(Entry {
            role: *role,
            kind,
            rights: rights.bits(),
            handle: handle.as_raw(),
        });
    }
    let record =
        super::startup::install_record(metadata, &entries, &mut resources.process.handles, objects)
            .unwrap();
    resources.context.rdi = record.as_raw();
    resources.context.rsi = 0;
}

fn now() -> u64 {
    kryo_hal::monotonic_time().expect("No block service clock")
}
fn pause(deadline: u64) {
    assert!(now() < deadline);
    unsafe { kryo_hal::arm_deadline(deadline.min(now() + 100_000)) }.unwrap();
    kryo_hal::idle_until_interrupt().unwrap();
    unsafe { kryo_hal::stop_timer() }.unwrap();
}
