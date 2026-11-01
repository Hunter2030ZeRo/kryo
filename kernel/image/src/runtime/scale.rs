use super::{CallStats, DirectMap, IpcRuntime, KernelMemory, KernelObject, KernelRegion};
use super::{Objects, RefCell, Tasks, launch, smp};
use crate::boot::{self, memory::BootFramePool};
use kryo_core::capability::Rights;
use kryo_core::object::ObjectKind;
use kryo_core::process::{ExitReason, ExitStatus};
use kryo_core::sched::TaskState;

/// Exercises growing production registries through native Ring 3 calls.
pub(crate) fn check(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let image = boot::module_image("kryo-scale-probe").expect("Missing scale probe image");
    let objects = Objects::growing();
    let mut tasks = Tasks::growing();
    let mut ipc = IpcRuntime::growing(&objects);
    let mut stats = CallStats::default();
    let first = region
        .find_stack(pool, direct)
        .expect("No initial scale stack");
    let mut initial = launch::prepare(pool, direct, memory, region, first, image)
        .unwrap_or_else(|_| panic!("Cannot load scale probe"));
    initial.context.rdi = initial
        .process
        .handles
        .insert(
            objects
                .create(ObjectKind::Image, KernelObject::Image(image))
                .unwrap(),
            Rights::EXECUTE,
        )
        .unwrap()
        .as_raw();
    initial.context.rsi = initial
        .process
        .handles
        .insert(
            objects
                .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
                .unwrap(),
            Rights::WRITE | Rights::TRANSFER,
        )
        .unwrap()
        .as_raw();
    tasks
        .reserve_process()
        .expect("Cannot reserve initial registry slots");
    let before_failure = pool.stats();
    crate::heap::fail_next();
    let kryo_core::task::SpawnError { error, resources } = tasks
        .spawn(initial)
        .expect_err("Membership allocation must fail");
    assert_eq!(error, kryo_core::task::TaskError::Capacity);
    assert_eq!(tasks.process_count(), 0);
    assert_eq!(tasks.live_count(), 0);
    assert_eq!(pool.stats(), before_failure);
    let parent = tasks
        .spawn(resources)
        .expect("Cannot retry scale publication");
    kryo_hal::emergency_write_str("SCALE: process publication OOM retained owners\n");

    // Each of six children owns twenty additional threads. All 127 threads
    // must register real timed waits after their TLS/shared-state checks.
    let mut ready = false;
    for _ in 0..16_384 {
        assert!(
            tasks.next_dying(None).is_none(),
            "Scale probe terminated early"
        );
        if tasks.live_count() == 127 && ipc.waits.live_count() == 127 {
            ready = tasks
                .ids()
                .all(|id| tasks.state(id) == Ok(TaskState::Blocked));
            if ready {
                break;
            }
        }
        assert!(
            tasks.has_runnable(),
            "Scale probe blocked before reaching capacity"
        );
        smp::step(&mut tasks, &mut stats, &mut ipc, region);
        assert!(
            !stats.last_debug.is_some_and(|(_, event, _)| event == 255),
            "Scale probe rejected a native operation"
        );
    }
    assert!(ready, "Scale probe did not finish registration");
    assert_eq!(tasks.process_count(), 7);
    assert_eq!(stats.spawned, 6);
    assert_eq!(stats.threads_created, 120);
    assert_eq!(objects.live_count(), 128);
    assert!(
        region.page_capacity() > 512,
        "Kernel stacks did not cross a leaf boundary"
    );
    let _ = kryo_hal::print(format_args!(
        "SCALE: native growth OK processes=7 threads=127 waits=127 stack_pages={}\n",
        region.page_capacity(),
    ));

    loop {
        let next = tasks
            .ids()
            .find(|id| tasks.state(*id) != Ok(TaskState::Dying));
        let Some(id) = next else { break };
        let process = tasks.process_for(id).unwrap();
        tasks
            .stop_process(
                process,
                ExitStatus {
                    reason: ExitReason::Stopped,
                    code: 0,
                },
            )
            .unwrap();
    }
    ipc.settle(&mut tasks);
    while let Some(id) = tasks.next_dying(None) {
        drop(
            tasks
                .reap(id, |owner| owner.close(pool, region))
                .expect("Cannot retire scale thread"),
        );
    }
    assert_eq!(tasks.live_count(), 0);
    assert_eq!(tasks.process_count(), 0);
    assert_eq!(ipc.waits.live_count(), 0);
    assert!(tasks.state(parent).is_err());
    drop(ipc);
    assert_eq!(objects.live_count(), 0);
    // Registry generations remain until their owners drop. The shared stack
    // branch retains extra table pages until the final global retirement.
    drop(tasks);
    drop(objects);
    kryo_hal::emergency_write_str("SCALE: native waits and process resources retired\n");
}
