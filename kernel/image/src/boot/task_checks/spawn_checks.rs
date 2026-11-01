use super::{
    BootFramePool, CallStats, DirectMap, IpcRuntime, KernelMemory, KernelObject, KernelRegion,
    Objects, Program, RefCell, Resources, Rights, TaskState, Tasks, programs,
};
use crate::runtime::{Object, launch, step_with_region};
use kryo_core::capability::Handle;
use kryo_core::object::ObjectKind;
use kryo_core::task::TaskId;
use kryo_hal::{PagingError, UserReason};
use kryo_interfaces::syscall::{Call, Error};

static MALFORMED: [u8; 64] = [0; 64];

#[derive(Clone, Copy, Debug)]
enum Case {
    ExecuteDenied,
    Malformed,
    HandlesFull,
    ObjectsFull,
    TasksFull,
}

struct PreparedTasks {
    parent: TaskId,
    peer: Option<TaskId>,
    handles: [Option<Handle>; 8],
    count: usize,
}

pub(super) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let worker = crate::boot::module_image("kryo-worker").expect("Missing spawn fixture image");
    warm_image(pool, direct, memory, region, worker);
    for case in [
        Case::ExecuteDenied,
        Case::Malformed,
        Case::HandlesFull,
        Case::ObjectsFull,
        Case::TasksFull,
    ] {
        run_case(case, pool, direct, memory, region, worker);
    }
    kryo_hal::emergency_write_str("SPAWN: publication rollback OK cases=5\n");
}

fn run_case(
    case: Case,
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
    worker: &'static [u8],
) {
    let before = pool.stats();
    let objects = Objects::new();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::with_objects(&objects);
    let mut stats = CallStats::default();
    let (expected, exit_code) = expectation(case);
    let bytes: &'static [u8] = if matches!(case, Case::Malformed) {
        &MALFORMED
    } else {
        worker
    };
    let image = objects
        .create(ObjectKind::Image, KernelObject::Image(bytes))
        .unwrap();
    let rights = if matches!(case, Case::ExecuteDenied) {
        Rights::INSPECT
    } else {
        Rights::EXECUTE
    };
    let PreparedTasks {
        parent,
        peer,
        handles,
        count,
    } = prepare_tasks(case, pool, direct, memory, region, &mut tasks, &image);
    let mut retained: [Option<Object<'_>>; 7] = [const { None }; 7];
    if matches!(case, Case::ObjectsFull) {
        for slot in &mut retained {
            *slot = Some(
                objects
                    .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
                    .unwrap(),
            );
        }
        assert_eq!(objects.live_count(), 8);
    }
    let allocated = pool.stats();
    let references = image.reference_count();
    let object_count = objects.live_count();
    let mut rejected = false;
    for _ in 0..32 {
        let (task, outcome) = step_with_region(&mut tasks, &mut stats, &mut ipc, region);
        let outcome = outcome.expect("Spawn failure fixture context rejected");
        assert_ne!(outcome.reason, UserReason::Fault);
        if task == parent && outcome.reason == UserReason::Call {
            assert_eq!(
                tasks.resources_mut(parent).unwrap().context.rax,
                expected.status(),
                "Spawn failure fixture: {:?}",
                case,
            );
            rejected = true;
            break;
        }
    }
    assert!(rejected, "Spawn fixture did not reach its syscall");
    assert_eq!(stats.spawned, 0);
    assert_eq!(tasks.live_count(), 1 + usize::from(peer.is_some()));
    assert_eq!(pool.stats(), allocated);
    assert_eq!(objects.live_count(), object_count);
    assert_eq!(image.reference_count(), references);
    let resources = tasks.resources_mut(parent).unwrap();
    assert!(resources.process.pending.is_none());
    assert_eq!(resources.process.handles.live_count(), count);
    assert_eq!(resources.process.handles.reserved_count(), 0);
    for handle in handles.into_iter().flatten() {
        let found = resources
            .process
            .handles
            .lookup(handle, ObjectKind::Image, rights)
            .unwrap();
        assert_eq!(found.id(), image.id());
    }
    if peer.is_none() {
        for slot in 6..=10 {
            assert_eq!(region.permissions(slot), Err(PagingError::NotMapped));
        }
    }
    if let Some(peer) = peer {
        tasks.stop(peer).unwrap();
        ipc.settle(&mut tasks);
        reap_task(&mut tasks, peer, pool, region);
    }
    for _ in 0..32 {
        if tasks.state(parent) == Ok(TaskState::Dying) {
            break;
        }
        let (task, outcome) = step_with_region(&mut tasks, &mut stats, &mut ipc, region);
        assert_eq!(task, parent);
        assert_ne!(outcome.unwrap().reason, UserReason::Fault);
    }
    assert_eq!(tasks.state(parent), Ok(TaskState::Dying));
    assert_eq!(tasks.resources_mut(parent).unwrap().exit_code, exit_code);
    reap_task(&mut tasks, parent, pool, region);
    assert_eq!(tasks.live_count(), 0);
    assert_eq!(ipc.waits.next_ready(), None);
    assert_eq!(ipc.waits.next_deadline(), None);
    assert_eq!(image.reference_count(), 1);
    drop(retained);
    drop(image);
    drop(ipc);
    assert_eq!(objects.live_count(), 0);
    assert_eq!(pool.stats(), before);
}

// Keep large setup/retirement values out of the syscall phase's debug stack
// frame. These phases finish before step_with_region enters the spawn path.
#[inline(never)]
fn warm_image(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
    worker: &'static [u8],
) {
    let mut prepared = match launch::prepare(pool, direct, memory, region, 2, worker) {
        Ok(prepared) => prepared,
        Err(mut failure) => {
            assert!(failure.pending.close(pool, region));
            panic!("Cannot warm spawn fixture mappings");
        }
    };
    prepared.close(pool, region).unwrap();
    drop(prepared);
}

#[inline(never)]
fn prepare_tasks<'kernel, 'objects>(
    case: Case,
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    tasks: &mut Tasks<'kernel, 'objects>,
    image: &Object<'objects>,
) -> PreparedTasks {
    let (expected, exit_code) = expectation(case);
    let mut parent = Resources::new(
        pool,
        direct,
        memory,
        region,
        2,
        &caller(expected, exit_code),
    );
    let rights = if matches!(case, Case::ExecuteDenied) {
        Rights::INSPECT
    } else {
        Rights::EXECUTE
    };
    let count = if matches!(case, Case::HandlesFull) {
        8
    } else {
        1
    };
    let mut handles = [None; 8];
    for slot in &mut handles[..count] {
        *slot = Some(
            parent
                .process
                .handles
                .insert(image.try_clone().unwrap(), rights)
                .unwrap(),
        );
    }
    parent.context.rbp = handles[0].unwrap().as_raw();
    let parent = tasks.spawn(parent).unwrap();
    let peer = if matches!(case, Case::TasksFull) {
        Some(
            tasks
                .spawn(Resources::new(
                    pool,
                    direct,
                    memory,
                    region,
                    7,
                    &programs::counter(),
                ))
                .unwrap(),
        )
    } else {
        None
    };
    PreparedTasks {
        parent,
        peer,
        handles,
        count,
    }
}

#[inline(never)]
fn reap_task(
    tasks: &mut Tasks<'_, '_>,
    task: TaskId,
    pool: &BootFramePool,
    region: &mut KernelRegion,
) {
    drop(
        tasks
            .reap(task, |resources| resources.close(pool, region))
            .unwrap(),
    );
}

fn expectation(case: Case) -> (Error, u64) {
    match case {
        Case::ExecuteDenied => (Error::PermissionDenied, 80),
        Case::Malformed => (Error::InvalidArgument, 81),
        Case::HandlesFull => (Error::OutOfResources, 82),
        Case::ObjectsFull => (Error::OutOfResources, 83),
        Case::TasksFull => (Error::OutOfResources, 84),
    }
}

fn caller(expected: Error, exit_code: u64) -> Program {
    let mut code = Program::new();
    code.emit(&[0xb8]);
    code.emit(&(Call::Spawn as u32).to_le_bytes());
    code.emit(&[0x48, 0x89, 0xef]);
    code.immediate(&[0x48, 0xbe], 0);
    code.emit(&[0xcd, 0x80]);
    code.expect_status(expected.status());
    code.expect_value(0);
    code.call(Call::Exit, exit_code, 0);
    code.finish()
}
