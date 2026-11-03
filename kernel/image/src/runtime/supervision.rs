use super::{
    BootFramePool, Handles, KernelMemory, KernelObject, KernelRegion, Objects, RefCell, Tasks,
};
#[cfg(feature = "boot-tests")]
use super::{CallStats, IpcRuntime};
use kryo_core::capability::Rights;
use kryo_core::ipc::ChannelBudget;
use kryo_core::object::ObjectKind;
#[cfg(feature = "boot-tests")]
use kryo_core::resource::Resource;
use kryo_core::resource::{Budget, Limits};
use kryo_core::task::TaskId;
use kryo_hal::DirectMap;
use kryo_interfaces::startup::{Entry, Metadata, ObjectKind as StartupKind};

pub(crate) struct Initial {
    pub(crate) task: TaskId,
    pub(crate) budgets: [ChannelBudget; 4],
    pub(crate) resources: [Budget; 4],
}

pub(crate) fn start<'kernel, 'objects>(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    objects: &'objects Objects,
    tasks: &mut Tasks<'kernel, 'objects>,
    argument: u64,
) -> Initial {
    let image = crate::boot::module_image("kryo-supervisor").expect("Missing supervisor image");
    let peer = crate::boot::module_image("kryo-echo-service").expect("Missing echo service image");
    let client =
        crate::boot::module_image("kryo-registry-client").expect("Missing registry client image");
    let accounts = [
        Budget::new(Limits {
            shared_pages: 64,
            private_pages: 0,
            reserved_pages: 0,
            handles: 128,
            waits: 16,
            threads: 16,
            mappings: 128,
            tables: 64,
        }),
        service_budget(),
        service_budget(),
        service_budget(),
    ];
    let mut resources = super::launch::prepare_budgeted(
        pool,
        direct,
        memory,
        region,
        2,
        image,
        Some(accounts[0].clone()),
    )
    .unwrap_or_else(|mut failure| {
        failure.pending.close(pool, region);
        panic!("Cannot prepare initial supervisor: {:?}", failure.error);
    });
    let budgets = [
        ChannelBudget::new(16),
        ChannelBudget::new(8),
        ChannelBudget::new(8),
        ChannelBudget::new(8),
    ];
    let mut handles = Handles::budgeted(accounts[0].clone());
    let mut entries = alloc::vec::Vec::new();
    for (index, role) in [2, 7, 8, 9].into_iter().enumerate() {
        let rights = if index == 0 {
            Rights::CREATE
        } else {
            Rights::CREATE | Rights::TRANSFER
        };
        let object = objects
            .create(
                ObjectKind::ChannelFactory,
                KernelObject::ChannelFactory(budgets[index].clone()),
            )
            .unwrap();
        let handle = handles.insert(object, rights).unwrap();
        entries.push(Entry {
            role,
            kind: StartupKind::ChannelFactory,
            rights: rights.bits(),
            handle: handle.as_raw(),
        });
    }
    for (role, image) in [(5, peer), (6, client)] {
        let object = objects
            .create(ObjectKind::Image, KernelObject::Image(image))
            .unwrap();
        let handle = handles.insert(object, Rights::EXECUTE).unwrap();
        entries.push(Entry {
            role,
            kind: StartupKind::Image,
            rights: Rights::EXECUTE.bits(),
            handle: handle.as_raw(),
        });
    }
    let rights = Rights::WRITE | Rights::TRANSFER;
    let debug = objects
        .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
        .unwrap();
    let handle = handles.insert(debug, rights).unwrap();
    entries.push(Entry {
        role: 3,
        kind: StartupKind::DebugConsole,
        rights: rights.bits(),
        handle: handle.as_raw(),
    });
    for (index, role) in [11, 12, 13].into_iter().enumerate() {
        let rights = Rights::READ | Rights::ASSIGN | Rights::TRANSFER;
        let object = objects
            .create(
                ObjectKind::ResourceBudget,
                KernelObject::ResourceBudget(accounts[index + 1].clone()),
            )
            .unwrap();
        let handle = handles.insert(object, rights).unwrap();
        entries.push(Entry {
            role,
            kind: StartupKind::ResourceBudget,
            rights: rights.bits(),
            handle: handle.as_raw(),
        });
    }
    let startup = super::startup::install_record(
        Metadata {
            service: 1,
            instance: 1,
            argument,
        },
        &entries,
        &mut handles,
        objects,
    )
    .expect("Cannot install supervisor startup authority");
    resources.context.rdi = startup.as_raw();
    resources.context.rsi = 0;
    resources.process.handles = handles;
    let task = tasks
        .spawn(resources)
        .expect("Cannot publish initial supervisor");
    Initial {
        task,
        budgets,
        resources: accounts,
    }
}

fn service_budget() -> Budget {
    Budget::new(Limits {
        shared_pages: 8,
        private_pages: 0,
        reserved_pages: 0,
        handles: 32,
        waits: 2,
        threads: 4,
        mappings: 64,
        tables: 32,
    })
}

#[cfg(feature = "boot-tests")]
pub(crate) fn check(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    super::budgets::checks::run(pool, direct, memory, region);
    super::startup::checks::run();
    super::startup::checks::spawn(pool, direct, memory, region);
    super::startup::receive_checks::run(pool, direct, memory, region);
    let before = pool.stats();
    let retained_before = memory.borrow().retained_pages();
    {
        let objects = Objects::growing();
        let mut tasks = Tasks::growing();
        let mut ipc = IpcRuntime::growing(&objects);
        let initial = start(pool, direct, memory, region, &objects, &mut tasks, 1);
        let process = tasks.process_for(initial.task).unwrap();
        let mut calls = CallStats::default();
        let mut result = None;
        let deadline = kryo_hal::monotonic_time()
            .unwrap()
            .saturating_add(10_000_000_000);
        for _ in 0..16384 {
            if kryo_hal::monotonic_time().unwrap() >= deadline {
                for id in tasks.ids() {
                    let thread = tasks.thread(id).unwrap();
                    let _ = kryo_hal::print(format_args!(
                        "SUPERVISOR watchdog: task={:#x} state={:?} rip={:#x} call={} wait={:?}\n",
                        id.as_raw(),
                        tasks.state(id),
                        thread.context.rip,
                        thread.context.rax,
                        thread.wait,
                    ));
                }
                panic!("Supervisor stalled");
            }
            ipc.settle(&mut tasks);
            if result.is_none() {
                result = tasks.terminal(process);
                if let Some(status) = result {
                    assert_eq!(
                        status,
                        kryo_core::process::ExitStatus {
                            reason: kryo_core::process::ExitReason::Exited,
                            code: 0,
                        },
                        "Supervisor rejected bootstrap; last native failure: {:?}; expired receive admissions={}",
                        calls.last_failure,
                        calls.expired_receive_admissions
                    );
                }
            }
            let mut cursor = None;
            while let Some(id) = tasks.next_dying(cursor) {
                cursor = Some(id);
                let retired = tasks
                    .reap(id, |resources| resources.close(pool, region))
                    .expect("Cannot retire supervised process");
                if retired.exit_code != 0 {
                    let _ = kryo_hal::print(format_args!(
                        "SUPERVISOR child: task={:#x} reason={:?} code={}\n",
                        id.as_raw(),
                        retired.exit_reason,
                        retired.exit_code,
                    ));
                }
                drop(retired);
            }
            ipc.settle(&mut tasks);
            if tasks.live_count() == 0 {
                break;
            }
            if tasks.has_runnable() {
                super::smp::step(&mut tasks, &mut calls, &mut ipc, region);
            } else if ipc.waits.next_deadline().is_some() {
                super::idle_until_deadline(&mut tasks, &mut ipc);
            } else {
                panic!("Supervisor has no wake source");
            }
        }
        assert_eq!(
            result,
            Some(kryo_core::process::ExitStatus {
                reason: kryo_core::process::ExitReason::Exited,
                code: 0,
            })
        );
        assert_eq!(calls.last_debug, Some((initial.task, 6, 2)));
        assert_eq!(calls.spawned, 5);
        assert_eq!(calls.threads_created, 6);
        assert_eq!(tasks.live_count(), 0);
        assert_eq!(objects.live_count(), 0);
        assert_eq!(ipc.waits.next_ready(), None);
        assert_eq!(ipc.waits.next_deadline(), None);
        for budget in &initial.budgets {
            assert_eq!(budget.live_count(), 0);
        }
        for budget in &initial.resources {
            for resource in [
                Resource::SharedPages,
                Resource::Handles,
                Resource::Waits,
                Resource::Threads,
            ] {
                assert_eq!(budget.count(resource), 0);
            }
        }
    }
    let retained = memory.borrow().retained_pages() - retained_before;
    assert_eq!(pool.stats().total, before.total);
    assert_eq!(pool.stats().free, before.free - retained);
    kryo_hal::emergency_write_str("SUPERVISOR: manifests and authority isolation OK instances=2\n");
}
