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
    let image = match crate::boot::module_image("kryo-abi-probe") {
        Ok(image) => image,
        Err(crate::boot::ModuleError::Missing) => return,
        Err(error) => panic!("Invalid native ABI probe module: {:?}", error),
    };
    let before = pool.stats();
    let retained = memory.borrow().retained_pages();
    {
        let objects = Objects::growing();
        let mut tasks = Tasks::growing();
        let mut ipc = IpcRuntime::growing(&objects);
        let account = Budget::new(Limits {
            shared_pages: 0,
            private_pages: 1024,
            reserved_pages: 65536,
            handles: 8,
            waits: 4,
            threads: 3,
            mappings: 4096,
            tables: 128,
        });
        let slot = region
            .find_stack(pool, direct)
            .expect("No ABI entry-stack cell");
        let mut resources = super::launch::prepare_native(
            pool,
            direct,
            memory,
            region,
            slot,
            super::launch::NativeImage {
                bytes: image,
                argv: &[b"abi-probe", b"cryolite-v3"],
                envp: &[b"KRYO_ABI=1"],
            },
            Some(account.clone()),
        )
        .unwrap_or_else(|mut failure| {
            failure.pending.close(pool, region);
            panic!("Cannot prepare native ABI probe: {:?}", failure.error);
        });
        assert!(
            resources.extended.is_some(),
            "Hard-float users require extended CPU state"
        );
        let debug = objects
            .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
            .unwrap();
        let debug = resources
            .process
            .handles
            .insert(debug, Rights::WRITE)
            .unwrap()
            .as_raw();
        let startup = super::startup::install_record(
            Metadata {
                service: 50,
                instance: 1,
                argument: 0,
            },
            &[Entry {
                role: 1,
                handle: debug,
                kind: WireKind::DebugConsole,
                rights: Rights::WRITE.bits(),
            }],
            &mut resources.process.handles,
            &objects,
        )
        .expect("Cannot install ABI startup capability");
        resources.context.rdi = startup.as_raw();
        resources.context.rsi = 0;
        let task = tasks
            .spawn(resources)
            .unwrap_or_else(|_| panic!("Cannot publish native ABI probe"));
        let owner = tasks.process_for(task).unwrap();
        let mut calls = CallStats::default();
        let mut start_ticks = None;
        let mut avx_preemptions = None;
        let mut tls_checked = false;
        let mut private_vm_checked = false;
        let mut words_checked = false;
        let mut words_halted = false;
        let mut retired = 0;
        let deadline = kryo_hal::monotonic_time().unwrap() + 5_000_000_000;
        while tasks.terminal(owner).is_none() {
            assert!(
                kryo_hal::monotonic_time().unwrap() < deadline,
                "Native ABI probe stalled"
            );
            super::expire_waits(&mut tasks, &mut ipc);
            if tasks.has_runnable() {
                super::smp::step(&mut tasks, &mut calls, &mut ipc, region);
            } else {
                let words_pending = !ipc.words.is_empty();
                words_halted |= super::idle_until_deadline(&mut tasks, &mut ipc) && words_pending;
            }
            if calls.last_debug == Some((task, 90, 0)) && start_ticks.is_none() {
                start_ticks = Some(tasks.thread_mut(task).unwrap().ticks);
            }
            if calls.last_debug == Some((task, 90, 1)) && avx_preemptions.is_none() {
                let ticks = tasks.thread_mut(task).unwrap().ticks;
                let before = start_ticks.expect("Missing AVX2 interval start");
                assert!(ticks > before, "AVX2 interval was not preempted");
                avx_preemptions = Some(ticks - before);
            }
            if calls.last_debug == Some((task, 94, 2)) {
                tls_checked = true;
            }
            if calls.last_debug == Some((task, 95, 1)) && !private_vm_checked {
                assert_eq!(account.count(Resource::PrivatePages), 4);
                assert_eq!(account.count(Resource::ReservedPages), 8);
                private_vm_checked = true;
            }
            if calls.last_debug == Some((task, 96, 1)) && !words_checked {
                assert_eq!(account.count(Resource::PrivatePages), 4);
                assert_eq!(account.count(Resource::ReservedPages), 8);
                words_checked = true;
            }
            // Joined threads release their execution resources while retained
            // control objects continue to supply their terminal join results.
            if tasks.terminal(owner).is_none() {
                while let Some(id) = tasks.next_dying(None) {
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
        assert_eq!(status.reason, ExitReason::Exited);
        assert_eq!(status.code, 0, "C/C++/Rust native ABI failed");
        assert_eq!(calls.last_debug, Some((task, 90, 2)));
        let preemptions = avx_preemptions.expect("Missing AVX2 preemption proof");
        assert!(tls_checked, "Missing static TLS thread isolation proof");
        assert!(private_vm_checked, "Missing private VM ownership proof");
        assert!(words_checked, "Missing atomic private wait proof");
        assert!(
            words_halted,
            "Private wait deadline did not halt the idle CPU"
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
        assert_eq!(
            retired,
            calls.threads_created + 1,
            "Native threads did not retire"
        );
        assert_eq!(retired, 11, "Native wait fixture thread count changed");
        assert!(
            ipc.words.is_empty(),
            "Private word registrations survived teardown"
        );
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
            assert_eq!(account.count(resource), 0);
        }
        kryo_hal::emergency_write_str("TLS: ELF template and thread isolation OK threads=3\n");
        let _ = kryo_hal::print(format_args!(
            "ABI: v3 SysV C C++ Rust stack and state OK preemptions={}\n",
            preemptions
        ));
    }
    assert_eq!(
        pool.stats().free + memory.borrow().retained_pages() - retained,
        before.free
    );
    kryo_hal::emergency_write_str("ABI: startup and execution resources retired\n");
    kryo_hal::emergency_write_str("VM: private reserve commit retirement OK\n");
    kryo_hal::emergency_write_str(
        "WAIT: private atomic park wake retirement OK threads=11 halt=true\n",
    );
}
