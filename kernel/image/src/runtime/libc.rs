use super::{
    BootFramePool, CallStats, IpcRuntime, KernelMemory, KernelObject, KernelRegion, Objects,
    RefCell, Tasks,
};
use kryo_core::capability::Rights;
use kryo_core::object::ObjectKind;
use kryo_core::process::ExitReason;
use kryo_core::resource::{Budget, Limits, Resource};
use kryo_core::sched::TaskState;
use kryo_hal::DirectMap;
use kryo_interfaces::startup::{Entry, Metadata, ObjectKind as WireKind};

pub(crate) fn check(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let image = match crate::boot::module_image("kryo-libc-probe") {
        Ok(image) => image,
        Err(crate::boot::ModuleError::Missing) => return,
        Err(error) => panic!("Invalid libc probe module: {:?}", error),
    };
    for instance in 1..=2 {
        run(pool, direct, memory, region, image, instance);
    }
    kryo_hal::emergency_write_str(
        "LIBC: static mlibc startup TLS allocator formatting OK runs=2\n",
    );
    kryo_hal::emergency_write_str("LIBC: process and backing retirement OK\n");
}

fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
    image: &'static [u8],
    instance: u64,
) {
    let before = pool.stats();
    let retained = memory.borrow().retained_pages();
    let heap_before = memory.borrow().transient_heap_usage();
    {
        let objects = Objects::growing();
        let mut tasks = Tasks::growing();
        let mut ipc = IpcRuntime::growing(&objects);
        let account = Budget::new(Limits {
            shared_pages: 0,
            private_pages: 1024,
            reserved_pages: 65536,
            handles: 16,
            waits: 8,
            threads: 4,
            mappings: 4096,
            tables: 128,
        });
        let slot = region
            .find_stack(pool, direct)
            .expect("No libc entry-stack cell");
        let mut resources = super::launch::prepare_native(
            pool,
            direct,
            memory,
            region,
            slot,
            super::launch::NativeImage {
                bytes: image,
                argv: &[b"libc-probe", b"cryolite-v3"],
                envp: &[b"KRYO_LIBC=1"],
            },
            Some(account.clone()),
        )
        .unwrap_or_else(|mut failure| {
            failure.pending.close(pool, region);
            panic!("Cannot prepare libc probe: {:?}", failure.error);
        });
        assert!(resources.extended.is_some(), "Libc requires user FP state");
        let debug = objects
            .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
            .unwrap();
        let debug = resources
            .process
            .handles
            .insert(debug, Rights::WRITE | Rights::DUP)
            .unwrap()
            .as_raw();
        let startup = super::startup::install_record(
            Metadata {
                service: 51,
                instance,
                argument: 0,
            },
            &[Entry {
                role: 1,
                handle: debug,
                kind: WireKind::DebugConsole,
                rights: (Rights::WRITE | Rights::DUP).bits(),
            }],
            &mut resources.process.handles,
            &objects,
        )
        .expect("Cannot install libc startup capability");
        resources.context.rdi = startup.as_raw();
        resources.context.rsi = 0;
        let task = tasks
            .spawn(resources)
            .unwrap_or_else(|_| panic!("Cannot publish libc probe"));
        let owner = tasks.process_for(task).unwrap();
        let mut calls = CallStats::default();
        let mut events = [false; 4];
        let mut retired = 0;
        let deadline = kryo_hal::monotonic_time()
            .unwrap()
            .saturating_add(10_000_000_000);
        while tasks.terminal(owner).is_none() {
            assert!(
                kryo_hal::monotonic_time().unwrap() < deadline,
                "Libc probe stalled"
            );
            super::expire_waits(&mut tasks, &mut ipc);
            if tasks.has_runnable() {
                super::smp::step(&mut tasks, &mut calls, &mut ipc, region);
            } else {
                let next = ipc.waits.next_deadline().unwrap_or(deadline).min(deadline);
                unsafe { kryo_hal::arm_deadline(next) }.expect("Cannot arm libc watchdog");
                kryo_hal::idle_until_interrupt().expect("Cannot idle libc probe");
                unsafe { kryo_hal::stop_timer() }.expect("Cannot stop libc watchdog");
                super::expire_waits(&mut tasks, &mut ipc);
            }
            if let Some((source, event, value)) = calls.last_debug.take() {
                assert_eq!(source, task, "Unexpected child libc diagnostic");
                assert_ne!(event, 209, "Libc probe failed at stage {}", value);
                let index = match event {
                    200..=202 => {
                        assert_eq!(value, 1, "Invalid libc completion event {}", event);
                        (event - 200) as usize
                    }
                    203 => {
                        assert_eq!(value, 2, "Libc allocator threads did not finish");
                        3
                    }
                    _ => panic!("Unexpected libc probe event {}", event),
                };
                assert!(!events[index], "Duplicate libc probe event {}", event);
                events[index] = true;
            }
            // Kernel thread resources can retire after execution handoff; the
            // retained control object still supplies actual native join results.
            if tasks.terminal(owner).is_none() {
                while let Some(id) = tasks.next_dying(None) {
                    assert_ne!(id, task, "Libc main thread exited before its children");
                    drop(
                        tasks
                            .reap(id, &mut ipc, |resources| resources.close(pool, region))
                            .unwrap(),
                    );
                    retired += 1;
                }
            }
        }
        let status = tasks.terminal(owner).unwrap();
        assert_eq!(status.reason, ExitReason::Exited, "Libc probe faulted");
        assert_eq!(status.code, 42, "Libc probe returned failure");
        assert!(
            events.into_iter().all(|seen| seen),
            "Missing libc completion event"
        );
        assert_eq!(
            calls.threads_created, 2,
            "Libc fixture thread count changed"
        );
        ipc.settle(&mut tasks);
        assert_eq!(tasks.state(task), Ok(TaskState::Dying));
        while let Some(id) = tasks.next_dying(None) {
            drop(
                tasks
                    .reap(id, &mut ipc, |resources| resources.close(pool, region))
                    .unwrap(),
            );
            retired += 1;
        }
        assert_eq!(retired, 3, "Libc threads did not retire");
        assert_eq!(tasks.live_count(), 0);
        assert!(ipc.words.is_empty(), "Libc private waits survived teardown");
        assert_eq!(ipc.waits.live_count(), 0);
        drop(ipc);
        assert_eq!(objects.live_count(), 0);
        for resource in [
            Resource::SharedPages,
            Resource::PrivatePages,
            Resource::ReservedPages,
            Resource::Handles,
            Resource::Waits,
            Resource::Threads,
        ] {
            assert_eq!(account.count(resource), 0, "Libc resource charge survived");
        }
    }
    assert_eq!(memory.borrow().transient_heap_usage(), heap_before);
    assert_eq!(pool.stats().total, before.total);
    assert_eq!(
        pool.stats().free + memory.borrow().retained_pages(),
        before.free + retained,
    );
}
