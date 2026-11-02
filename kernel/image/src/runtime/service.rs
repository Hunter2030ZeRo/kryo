use super::{CallStats, DirectMap, IpcRuntime, KernelMemory, KernelRegion};
#[cfg(feature = "boot-tests")]
use super::{Handles, KernelObject};
use super::{Objects, RefCell, Tasks, smp};
use crate::boot::memory::BootFramePool;
#[cfg(feature = "boot-tests")]
use kryo_core::capability::Rights;
#[cfg(feature = "boot-tests")]
use kryo_core::object::ObjectKind;
#[cfg(feature = "boot-tests")]
use kryo_core::task::TaskId;

#[cfg(not(feature = "boot-tests"))]
pub(crate) fn run(pool: &'static BootFramePool, direct: DirectMap, physical_end: u64) -> ! {
    let mut region = crate::boot::platform::take_region();
    let memory = RefCell::new(
        unsafe { KernelMemory::new(pool, direct, physical_end, &region) }
            .expect("Cannot establish process alias policy"),
    );
    crate::boot::storage::check_buffers(pool, direct, &memory);
    let mmio = crate::boot::platform::apic_mapping();
    super::clock::initialize(&mut region);
    smp::start(mmio);
    super::transport::run(pool, direct, &memory, &mut region);
    super::devices::probe(pool, direct, &memory, &mut region, false);
    super::channel_probe::run(pool, direct, &memory, &mut region);
    let objects = Objects::growing();
    let mut tasks = Tasks::growing();
    let mut ipc = IpcRuntime::growing(&objects);
    let mut stats = CallStats::default();
    let initial =
        super::supervision::start(pool, direct, &memory, &mut region, &objects, &mut tasks, 0);
    let supervisor = initial.task;
    let supervisor_process = tasks
        .process_for(supervisor)
        .expect("Supervisor process membership invariant");
    let _ = kryo_hal::print(format_args!(
        "BOOT: supervisor dispatched id={:#x} accounts={} budgets={}\n",
        supervisor.as_raw(),
        initial.budgets.len(),
        initial.resources.len()
    ));
    loop {
        reap(&mut tasks, &mut ipc, pool, &mut region);
        if tasks.process_thread_count(supervisor_process).is_err()
            || tasks.terminal(supervisor_process).is_some()
        {
            let _ = kryo_hal::print(format_args!(
                "BOOT: supervisor status={:?}\n",
                tasks.terminal(supervisor_process)
            ));
            kryo_hal::emergency_write_str(
                "BOOT: supervisor terminated; no kernel restart policy\n",
            );
            halt();
        }
        if tasks.has_runnable() {
            let _batch = smp::step(&mut tasks, &mut stats, &mut ipc, &mut region);
        } else if ipc.waits.next_deadline().is_some() || super::devices::has_wake_source(&ipc) {
            super::idle_for_events(&mut tasks, &mut ipc);
        } else {
            kryo_hal::emergency_write_str("BOOT: no registered wake source; runtime stopped\n");
            halt();
        }
    }
}

#[cfg(feature = "boot-tests")]
fn start<'kernel, 'objects>(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    objects: &'objects Objects,
    tasks: &mut Tasks<'kernel, 'objects>,
) -> TaskId {
    let image = crate::boot::module_image("kryo-init").expect("Missing init image");
    let worker = crate::boot::module_image("kryo-worker").expect("Missing worker image");
    let mut initial = super::launch::prepare(pool, direct, memory, region, 2, image)
        .unwrap_or_else(|mut failure| {
            failure.pending.close(pool, region);
            panic!("Cannot prepare initial process: {:?}", failure.error);
        });
    let mut handles = Handles::growing();
    let worker = objects
        .create(ObjectKind::Image, KernelObject::Image(worker))
        .unwrap();
    let debug = objects
        .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
        .unwrap();
    initial.context.rdi = handles.insert(worker, Rights::EXECUTE).unwrap().as_raw();
    initial.context.rsi = handles
        .insert(debug, Rights::WRITE | Rights::TRANSFER)
        .unwrap()
        .as_raw();
    initial.process.handles = handles;
    let control = objects
        .create(
            ObjectKind::ProcessControl,
            KernelObject::ProcessControl(RefCell::new(super::ProcessControl::new())),
        )
        .unwrap();
    initial.process.control = Some(control.try_clone().unwrap());
    let init = tasks.spawn(initial).expect("Cannot publish init process");
    super::process::bind(&control, init, tasks.process_for(init).unwrap());
    init
}

#[derive(Default)]
struct ReapStats {
    created: usize,
    preempted_created: usize,
    preempted_main: usize,
}

fn reap<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    pool: &BootFramePool,
    region: &mut KernelRegion,
) -> ReapStats {
    let mut stats = ReapStats::default();
    ipc.settle(tasks);
    super::devices::retire_bindings(tasks, ipc)
        .expect("Device retirement failed; execution resources retained");
    let mut cursor = None;
    while let Some(id) = tasks.next_dying(cursor) {
        cursor = Some(id);
        // A failed cleanup keeps the Dying slot and its owners for retry. Exit
        // observers may already hold the immutable terminal result.
        if let Ok(resources) = tasks.reap(id, |resources| resources.close(pool, region)) {
            if resources.thread_control.is_some() {
                stats.created += 1;
                stats.preempted_created += usize::from(resources.ticks > 0);
            } else {
                stats.preempted_main += usize::from(resources.ticks > 0);
            }
            drop(resources);
        }
    }
    stats
}

#[cfg(feature = "boot-tests")]
pub(crate) fn check(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let objects = Objects::new();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::with_objects(&objects);
    let mut stats = CallStats::default();
    let init = start(pool, direct, memory, region, &objects, &mut tasks);
    let mut ready = false;
    let mut reaped = ReapStats::default();
    for _ in 0..1024 {
        let summary = reap(&mut tasks, &mut ipc, pool, region);
        reaped.created += summary.created;
        reaped.preempted_created += summary.preempted_created;
        reaped.preempted_main += summary.preempted_main;
        assert!(
            tasks.state(init).is_ok(),
            "Init terminated before supervision finished"
        );
        if tasks.has_runnable() {
            let _batch = smp::step(&mut tasks, &mut stats, &mut ipc, region);
        } else {
            super::idle_until_deadline(&mut tasks, &mut ipc);
        }
        if stats.last_debug == Some((init, 3, 2)) {
            ready = true;
            break;
        }
    }
    assert!(
        ready && stats.spawned == 2,
        "User-space supervision did not complete"
    );
    assert_eq!(stats.threads_created, 4);
    assert_eq!(reaped.created, 4);
    assert_eq!(reaped.preempted_created, 4);
    assert!(reaped.preempted_main >= 2);
    let _ = kryo_hal::print(format_args!(
        "THREAD: shared-process state OK threads={} preempted={}\n",
        reaped.created, reaped.preempted_created,
    ));
    assert_eq!(tasks.live_count(), 1);
    let mut resources = tasks.resources_mut(init).unwrap();
    resources.exit_reason = kryo_core::process::ExitReason::Stopped;
    resources.exit_code = 0;
    tasks.stop(init).unwrap();
    reap(&mut tasks, &mut ipc, pool, region);
    assert_eq!(tasks.live_count(), 0);
    assert_eq!(ipc.waits.next_ready(), None);
    assert_eq!(ipc.waits.next_deadline(), None);
    drop(ipc);
    assert_eq!(objects.live_count(), 0);
    assert_eq!(pool.stats(), before);
    kryo_hal::emergency_write_str("INIT: user supervision and restart OK generations=2\n");
}

#[cfg(not(feature = "boot-tests"))]
fn halt() -> ! {
    loop {
        unsafe { core::arch::asm!("cli", "hlt", options(nostack)) }
    }
}
