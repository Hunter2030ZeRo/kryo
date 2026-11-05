use super::{
    BootFramePool, CallStats, IpcRuntime, KernelMemory, KernelObject, KernelRegion, Objects,
    RefCell, Tasks,
};
use kryo_core::capability::Rights;
use kryo_core::object::ObjectKind;
use kryo_core::process::ExitReason;
use kryo_core::resource::{Budget, Limits, Resource};
use kryo_hal::DirectMap;
use kryo_interfaces::startup::{Entry, Metadata, ObjectKind as WireKind};

pub(crate) fn check(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let image = match crate::boot::module_image("kryo-rust-std-probe") {
        Ok(image) => image,
        Err(crate::boot::ModuleError::Missing) => return,
        Err(error) => panic!("Invalid Rust std probe module: {:?}", error),
    };
    let random_available = kryo_hal::cpu_random_available();
    for instance in 1..=3 {
        run(
            pool,
            direct,
            memory,
            region,
            image,
            instance,
            random_available,
        );
    }
    kryo_hal::emergency_write_str("RUSTSTD: native abort termination OK\n");
    kryo_hal::emergency_write_str(
        "RUSTSTD: allocator threads synchronization TLS monotonic time OK runs=2\n",
    );
    kryo_hal::emergency_write_str("RUSTSTD: process and backing retirement OK\n");
    kryo_hal::emergency_write_str("RUSTSTDFILE: read write seek clones and closure OK runs=2\n");
    kryo_hal::emergency_write_str("RUSTSTDFILEOPS: concurrent aliases OK runs=1\n");
    kryo_hal::emergency_write_str("RUSTSTDFILEOPS: wait quota rejection and recovery OK runs=1\n");
    kryo_hal::emergency_write_str(
        "RUSTSTDFILEOPS: metadata positioned I/O resize and sync rejection OK runs=2\n",
    );
    kryo_hal::emergency_write_str(if random_available {
        "RUSTSTDRANDOM: CPU source and HashMap OK runs=2\n"
    } else {
        "RUSTSTDRANDOM: unavailable source rejected OK runs=2\n"
    });
}

fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
    image: &'static [u8],
    instance: u64,
    random_available: bool,
) {
    let abort = instance == 3;
    let before = pool.stats();
    let retained = memory.borrow().retained_pages();
    let heap_before = memory.borrow().transient_heap_usage();
    {
        let objects = Objects::growing();
        let mut tasks = Tasks::growing();
        let mut ipc = IpcRuntime::growing(&objects);
        let account = Budget::new(Limits {
            shared_pages: if abort { 0 } else { 8 },
            private_pages: 8192,
            reserved_pages: 65536,
            handles: 16,
            waits: if instance == 2 { 1 } else { 8 },
            threads: 4,
            mappings: 16384,
            tables: 256,
        });
        let slot = region
            .find_stack(pool, direct)
            .expect("No Rust std entry-stack cell");
        let mut resources = super::launch::prepare_native(
            pool,
            direct,
            memory,
            region,
            slot,
            super::launch::NativeImage {
                bytes: image,
                argv: if abort {
                    &[b"rust-std-probe", b"abort"]
                } else if instance == 1 {
                    &[b"rust-std-probe", b"cryolite-v3", b"concurrent"]
                } else {
                    &[b"rust-std-probe", b"cryolite-v3", b"quota"]
                },
                envp: &[],
            },
            Some(account.clone()),
        )
        .unwrap_or_else(|mut failure| {
            failure.pending.close(pool, region);
            panic!("Cannot prepare Rust std probe: {:?}", failure.error);
        });
        assert!(
            resources.extended.is_some(),
            "Rust std requires user FP state"
        );
        let debug = objects
            .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
            .unwrap();
        let debug = resources
            .process
            .handles
            .insert(debug, Rights::WRITE | Rights::DUP)
            .unwrap()
            .as_raw();
        let (service, namespace) = if abort {
            (None, 0)
        } else {
            let (service, namespace) = super::libc::files::prepare(
                pool, direct, memory, region, &mut tasks, &mut ipc, &objects,
            );
            let namespace = resources
                .process
                .handles
                .insert(namespace, Rights::SEND)
                .unwrap()
                .as_raw();
            (Some(service), namespace)
        };
        let entries = [
            Entry {
                role: 1,
                handle: debug,
                kind: WireKind::DebugConsole,
                rights: (Rights::WRITE | Rights::DUP).bits(),
            },
            Entry {
                role: kryo_interfaces::file::NAMESPACE_ROLE,
                handle: namespace,
                kind: WireKind::EndpointGate,
                rights: Rights::SEND.bits(),
            },
        ];
        let startup = super::startup::install_record(
            Metadata {
                service: 52,
                instance,
                argument: 0,
            },
            &entries[..if abort { 1 } else { 2 }],
            &mut resources.process.handles,
            &objects,
        )
        .expect("Cannot install Rust std startup capability");
        resources.context.rdi = startup.as_raw();
        resources.context.rsi = 0;
        let task = tasks
            .spawn(resources)
            .unwrap_or_else(|_| panic!("Cannot publish Rust std probe"));
        let owner = tasks.process_for(task).unwrap();
        if let Some(service) = &service {
            tasks.wake(service.task).unwrap();
        }
        let mut calls = CallStats::default();
        let mut events = [false; 5];
        let mut files_seen = false;
        let mut metadata_seen = false;
        let mut file_ops_seen = false;
        let mut concurrent_files_seen = false;
        let mut file_quota_rejected = false;
        let mut file_quota_recovered = false;
        let mut random_seen = false;
        let mut abort_seen = false;
        let mut contention_started = false;
        let mut contention_finished = false;
        let mut mutex_wait_seen = false;
        let mut status = None;
        let mut service_status = None;
        let mut retired = 0;
        let deadline = kryo_hal::monotonic_time()
            .unwrap()
            .saturating_add(30_000_000_000);
        while status.is_none() || (service.is_some() && service_status.is_none()) {
            assert!(
                kryo_hal::monotonic_time().unwrap() < deadline,
                "Rust std probe stalled"
            );
            super::expire_waits(&mut tasks, &mut ipc);
            if tasks.has_runnable() {
                super::smp::step(&mut tasks, &mut calls, &mut ipc, region);
            } else {
                let next = ipc.waits.next_deadline().unwrap_or(deadline).min(deadline);
                unsafe { kryo_hal::arm_deadline(next) }.expect("Cannot arm Rust std watchdog");
                kryo_hal::idle_until_interrupt().expect("Cannot idle Rust std probe");
                unsafe { kryo_hal::stop_timer() }.expect("Cannot stop Rust std watchdog");
                super::expire_waits(&mut tasks, &mut ipc);
            }
            if contention_started && !contention_finished && !ipc.words.is_empty() {
                mutex_wait_seen = true;
            }
            if let Some((source, event, value)) = calls.last_debug.take() {
                assert_ne!(
                    event,
                    228,
                    "Rust std bridge failed: phase={} status={}",
                    value >> 32,
                    value as u32 as i32
                );
                assert_eq!(source, task, "Unexpected child Rust std diagnostic");
                #[cfg(feature = "boot-tests")]
                if event == 229 {
                    panic!(
                        "Rust std failed at stage {}: last native failure {:?}",
                        value, calls.last_failure
                    );
                }
                assert_ne!(event, 229, "Rust std probe failed at stage {}", value);
                if event != 231 {
                    assert_eq!(value, 1, "Invalid Rust std completion event {}", event);
                }
                match event {
                    220..=224 if !abort => {
                        let index = (event - 220) as usize;
                        assert!(!events[index], "Duplicate Rust std event {}", event);
                        events[index] = true;
                    }
                    225 if !abort => {
                        assert!(!contention_started, "Duplicate Rust std contention start");
                        contention_started = true;
                    }
                    226 if !abort => {
                        assert!(contention_started && !contention_finished);
                        assert!(mutex_wait_seen, "Rust std mutex slow path was not observed");
                        // The main thread still holds the mutex and is running this call.
                        // Both children entered their lock attempts before this event.
                        assert!(
                            !ipc.words.is_empty(),
                            "Rust std mutex children did not park"
                        );
                        contention_finished = true;
                    }
                    227 if abort => {
                        assert!(!abort_seen, "Duplicate Rust std abort event");
                        abort_seen = true;
                    }
                    230 if !abort => {
                        assert!(!files_seen, "Duplicate Rust std file event");
                        files_seen = true;
                    }
                    231 if !abort => {
                        assert!(!random_seen, "Duplicate Rust std random event");
                        assert_eq!(value, if random_available { 1 } else { 2 });
                        random_seen = true;
                    }
                    232 if !abort => {
                        assert!(!metadata_seen, "Duplicate Rust metadata event");
                        metadata_seen = true;
                    }
                    233 if !abort => {
                        assert!(metadata_seen, "Positioned I/O preceded metadata checks");
                        assert!(!file_ops_seen, "Duplicate Rust file operations event");
                        file_ops_seen = true;
                    }
                    234 if instance == 1 => {
                        assert!(!concurrent_files_seen, "Duplicate concurrent file event");
                        concurrent_files_seen = true;
                    }
                    235 if instance == 2 => {
                        assert!(!file_quota_rejected, "Duplicate file quota rejection");
                        assert_eq!(account.count(Resource::Waits), 1);
                        assert_eq!(account.count(Resource::SharedPages), 0);
                        #[cfg(feature = "boot-tests")]
                        assert_eq!(
                            calls.last_failure,
                            Some((
                                task,
                                kryo_interfaces::syscall::Call::CallExchange,
                                kryo_interfaces::syscall::Error::OutOfResources
                            ))
                        );
                        file_quota_rejected = true;
                    }
                    236 if instance == 2 => {
                        assert!(file_quota_rejected && !file_quota_recovered);
                        assert_eq!(account.count(Resource::Waits), 0);
                        assert_eq!(account.count(Resource::SharedPages), 0);
                        file_quota_recovered = true;
                    }
                    _ => panic!("Unexpected Rust std event {}", event),
                }
            }
            if status.is_none() {
                status = tasks.terminal(owner);
            }
            if let Some(service) = &service
                && service_status.is_none()
            {
                service_status = tasks.terminal(service.owner);
                if let Some(result) = service_status {
                    assert_eq!(result.reason, ExitReason::Exited, "Std file peer faulted");
                    assert_eq!(result.code, 0, "Std file peer failed");
                }
            }
            while let Some(id) = tasks.next_dying(None) {
                if status.is_none() {
                    assert_ne!(id, task, "Rust std main exited before its children");
                }
                drop(
                    tasks
                        .reap(id, &mut ipc, |resources| resources.close(pool, region))
                        .unwrap(),
                );
                retired += 1;
            }
        }
        let status = status.unwrap();
        assert_eq!(status.reason, ExitReason::Exited, "Rust std probe faulted");
        assert_eq!(
            status.code,
            if abort { 134 } else { 0 },
            "Rust std probe returned failure"
        );
        if abort {
            assert!(abort_seen, "Missing Rust std abort event");
        } else {
            assert!(
                events.into_iter().all(|seen| seen),
                "Missing Rust std completion event"
            );
            assert!(
                contention_finished,
                "Rust std contention proof did not finish"
            );
            assert!(files_seen, "Missing Rust std file completion event");
            assert!(
                metadata_seen && file_ops_seen,
                "Missing Rust file operations event"
            );
            assert_eq!(concurrent_files_seen, instance == 1);
            assert_eq!(file_quota_rejected && file_quota_recovered, instance == 2);
            assert!(random_seen, "Missing Rust std random completion event");
        }
        assert_eq!(
            calls.threads_created,
            if abort {
                0
            } else if instance == 1 {
                7
            } else {
                6
            },
            "Rust std fixture thread count changed"
        );
        ipc.settle(&mut tasks);
        assert_eq!(
            retired,
            if abort {
                1
            } else if instance == 1 {
                9
            } else {
                8
            },
            "Rust std threads did not retire"
        );
        if let Some(service) = &service {
            assert_eq!(service.channels.live_count(), 0, "Std file gates survived");
        }
        assert_eq!(tasks.live_count(), 0);
        assert!(
            ipc.words.is_empty(),
            "Rust std private waits survived teardown"
        );
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
            assert_eq!(
                account.count(resource),
                0,
                "Rust std resource charge survived"
            );
        }
    }
    assert_eq!(memory.borrow().transient_heap_usage(), heap_before);
    assert_eq!(pool.stats().total, before.total);
    assert_eq!(
        pool.stats().free + memory.borrow().retained_pages(),
        before.free + retained
    );
}
