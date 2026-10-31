mod ipc_checks;
mod ipc_programs;
mod ipc_runtime;
mod native;
mod programs;
mod stacks;

use super::memory::BootFramePool;
use super::vm_backend::{KernelMemory, UserSpace, new_space};
use core::cell::RefCell;
use ipc_runtime::IpcRuntime;
use kryo_core::capability::{HandleTable, Rights};
use kryo_core::ipc::{Endpoint, Notification};
use kryo_core::mm::vm::Permissions;
use kryo_core::object::{ObjectKind, ObjectRef, ObjectTable};
use kryo_core::sched::{ReturnReason, TaskState};
use kryo_core::sync::wait::WaitId;
use kryo_core::task::{TaskId, TaskTable};
use kryo_hal::{DirectMap, KernelRegion, UserContext, UserEntryError, UserOutcome, UserReason};
use programs::{CODE, DATA, Program, SEED_A, SEED_B, STACK};
use stacks::KernelStack;

const REGION_BASE: u64 = 0xffff_ff00_0000_0000;
const OBJECTS: usize = 8;
const HANDLES: usize = 8;
type Objects = ObjectTable<KernelObject, OBJECTS>;
type Object<'objects> = ObjectRef<'objects, KernelObject, OBJECTS>;
type Handles<'objects> = HandleTable<'objects, KernelObject, OBJECTS, HANDLES>;
type Tasks<'kernel, 'objects> = TaskTable<Resources<'kernel, 'objects>, 2>;

enum KernelObject {
    TaskControl(TaskId),
    Endpoint(RefCell<Endpoint<2, 2>>),
    Notification(RefCell<Notification>),
}

struct Resources<'kernel, 'objects> {
    space: UserSpace<'kernel>,
    context: UserContext,
    stack: KernelStack,
    handles: Handles<'objects>,
    ticks: usize,
    exit_code: u64,
    space_closed: bool,
    wait: Option<WaitId>,
}

#[derive(Default)]
struct CallStats {
    yields: usize,
    denied: usize,
    invalid: usize,
}

pub(super) fn run(pool: &'static BootFramePool, direct: DirectMap, kernel: &RefCell<KernelMemory>) {
    let before = pool.stats();
    let tables = [
        pool.allocate().expect("No first kernel-region table"),
        pool.allocate().expect("No second kernel-region table"),
        pool.allocate().expect("No leaf kernel-region table"),
    ];
    let physical = [
        tables[0].address(),
        tables[1].address(),
        tables[2].address(),
    ];
    // The region is installed before any task roots inherit the kernel half.
    let mut region = unsafe { KernelRegion::new(direct, REGION_BASE, physical) }
        .expect("Cannot attach kernel task region");
    let controller = kryo_hal::timer_physical_address().expect("No supported local APIC");
    let mmio = unsafe { region.map_mmio(0, controller) }.expect("Cannot map local APIC");
    let flags = region.permissions(0).expect("Missing APIC mapping");
    assert!(flags.writable && flags.uncached && !flags.executable && !flags.user);
    unsafe { kryo_hal::init_timer(mmio) }.expect("Cannot initialize local timer");

    check_preemption(pool, direct, kernel, &mut region);
    check_capabilities(pool, direct, kernel, &mut region);
    check_invalid_resume(pool, direct, kernel, &mut region);
    ipc_checks::run(pool, direct, kernel, &mut region);

    // Force a queued timer while IRQs are off. Shutdown must acknowledge it on
    // the kernel stack before the controller mapping can be removed.
    unsafe { kryo_hal::arm_timer_once(100_000) }.expect("Cannot queue shutdown timer");
    for _ in 0..100_000 {
        if kryo_hal::timer_pending() {
            break;
        }
        core::hint::spin_loop();
    }
    assert!(kryo_hal::timer_pending(), "Timer did not become pending");
    unsafe { kryo_hal::shutdown_timer() }.expect("Cannot drain and restore local timer");
    assert!(!kryo_hal::timer_initialized());
    kryo_hal::emergency_write_str("TIMER: pending interrupt drained\n");
    // All task roots and entry-stack accesses are retired; IRQs remain off.
    unsafe {
        assert_eq!(region.unmap(0).expect("Cannot unmap APIC"), controller);
        region.detach().expect("Cannot detach kernel task region");
        for frame in tables {
            pool.release(frame).expect("Cannot free kernel task tables");
        }
    }
    assert_eq!(pool.stats(), before);
    kryo_hal::emergency_write_str("TASK: teardown OK\n");
}

impl<'kernel, 'objects> Resources<'kernel, 'objects> {
    fn new(
        pool: &'static BootFramePool,
        direct: DirectMap,
        kernel: &'kernel RefCell<KernelMemory>,
        region: &mut KernelRegion,
        first_slot: usize,
        program: &Program,
    ) -> Self {
        let mut space = new_space(pool, kernel).expect("Cannot create task address space");
        space
            .map_page(CODE, Permissions::ReadExecute, program.as_bytes())
            .expect("Cannot map task code");
        space
            .map_page(DATA, Permissions::ReadWrite, &[])
            .expect("Cannot map task data");
        space
            .map_page(STACK, Permissions::ReadWrite, &[])
            .expect("Cannot map task user stack");
        let mut context =
            UserContext::new(CODE, STACK + 4096).expect("Invalid initial task context");
        context.rbx = DATA;
        context.r12 = SEED_A;
        context.r13 = SEED_B;
        Self {
            space,
            context,
            stack: KernelStack::new(pool, direct, region, first_slot),
            handles: Handles::new(),
            ticks: 0,
            exit_code: u64::MAX,
            space_closed: false,
            wait: None,
        }
    }

    fn close(
        &mut self,
        pool: &BootFramePool,
        region: &mut KernelRegion,
    ) -> Result<(), &'static str> {
        if !self.space_closed {
            self.space
                .close()
                .map_err(|_| "Cannot close task address space")?;
            self.space_closed = true;
        }
        // run_context restored TSS.RSP0 and the boot stack before returned().
        unsafe { self.stack.close(pool, region) }.map_err(|_| "Cannot close task entry stack")
    }
}

fn step<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    calls: &mut CallStats,
    ipc: &mut IpcRuntime<'objects>,
) -> (TaskId, Result<UserOutcome, UserEntryError>) {
    let id = tasks
        .dispatch()
        .expect("Scheduler already owns a running task")
        .expect("No runnable test task");
    let resources = tasks.resources_mut(id).expect("Missing running task");
    let top = resources.stack.top();
    let context = &mut resources.context;
    // The task borrow pins its private user root and guarded entry stack. The
    // HAL restores the boot root and saves all registers before this returns.
    let outcome = resources
        .space
        .with_root(|root| unsafe { kryo_hal::run_context(root, context, top, true) })
        .expect("Task address space still active");
    let reason = match outcome {
        Ok(UserOutcome {
            reason: UserReason::Preempted,
            ..
        }) => {
            resources.ticks += 1;
            ReturnReason::Preempted
        }
        Ok(UserOutcome {
            reason: UserReason::Call,
            ..
        }) => ReturnReason::Call,
        Ok(UserOutcome {
            reason: UserReason::Fault,
            vector,
            ..
        }) => {
            resources.exit_code = 0x100 + vector;
            ReturnReason::Fault
        }
        Ok(UserOutcome {
            reason: UserReason::Exited,
            status,
            ..
        }) => {
            resources.exit_code = status;
            ReturnReason::Exit
        }
        Err(error) => {
            let _ = kryo_hal::print(format_args!("TASK rejected: {:?}\n", error));
            resources.exit_code = 255;
            ReturnReason::Fault
        }
    };
    tasks.returned(id, reason).expect("Invalid task return");
    if matches!(
        outcome,
        Ok(UserOutcome {
            reason: UserReason::Call,
            ..
        })
    ) {
        native::dispatch(id, tasks, calls, ipc);
    }
    ipc.settle(tasks);
    (id, outcome)
}

fn check_progress(resources: &Resources<'_, '_>) {
    assert_eq!(resources.context.r12, SEED_A);
    assert_eq!(resources.context.r13, SEED_B);
    let mut bytes = [0; 8];
    resources
        .space
        .read_page(DATA, 0, &mut bytes)
        .expect("Cannot inspect task progress");
    let count = u64::from_le_bytes(bytes);
    assert!(count > 0 && resources.ticks > 0);
    assert!(count.abs_diff(resources.context.r14) <= 1);
    assert!(resources.context.r10.abs_diff(resources.context.r14) <= 1);
}

fn check_preemption(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let mut tasks = Tasks::new();
    let first = tasks
        .spawn(Resources::new(
            pool,
            direct,
            kernel,
            region,
            2,
            &programs::counter(),
        ))
        .unwrap();
    let second = tasks
        .spawn(Resources::new(
            pool,
            direct,
            kernel,
            region,
            7,
            &programs::counter(),
        ))
        .unwrap();
    let mut calls = CallStats::default();
    let mut ipc = IpcRuntime::new();
    for _ in 0..8 {
        let (_, outcome) = step(&mut tasks, &mut calls, &mut ipc);
        let outcome = outcome.expect("Counter context was rejected");
        assert_eq!(outcome.reason, UserReason::Preempted);
        assert_eq!(outcome.vector, 0xe0);
    }
    assert_eq!(calls.yields, 0);
    for id in [first, second] {
        let resources = tasks.resources_mut(id).unwrap();
        assert_eq!(resources.ticks, 4);
        check_progress(resources);
        tasks.stop(id).unwrap();
        drop(
            tasks
                .reap(id, |resources| resources.close(pool, region))
                .unwrap(),
        );
    }
    assert_eq!(tasks.live_count(), 0);
    kryo_hal::emergency_write_str("TIMER: preemption OK switches=8\n");
}

fn check_capabilities(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let objects = Objects::new();
    let mut tasks = Tasks::new();
    let target = tasks
        .spawn(Resources::new(
            pool,
            direct,
            kernel,
            region,
            2,
            &programs::counter(),
        ))
        .unwrap();
    let control = objects
        .create(ObjectKind::TaskControl, KernelObject::TaskControl(target))
        .unwrap();
    let inspect_rights = Rights::INSPECT | Rights::DUP;
    let control_rights = Rights::INSPECT | Rights::CONTROL | Rights::DUP;
    let mut handles = Handles::new();
    let inspect = handles
        .insert(control.try_clone().unwrap(), inspect_rights)
        .unwrap();
    let full = handles
        .insert(control.try_clone().unwrap(), control_rights)
        .unwrap();
    let code = programs::controller(
        inspect.as_raw(),
        full.as_raw(),
        Rights::INSPECT.bits(),
        Rights::CONTROL.bits(),
    );
    let mut controller = Resources::new(pool, direct, kernel, region, 7, &code);
    controller.handles = handles;
    let caller = tasks.spawn(controller).unwrap();
    let mut calls = CallStats::default();
    let mut ipc = IpcRuntime::new();
    let mut target_reaped = false;
    let mut caller_reaped = false;
    for _ in 0..128 {
        if tasks.live_count() == 0 {
            break;
        }
        let (_, outcome) = step(&mut tasks, &mut calls, &mut ipc);
        let outcome = outcome.expect("Capability test context was rejected");
        assert_ne!(outcome.reason, UserReason::Fault);
        for id in [target, caller] {
            if tasks.state(id) != Ok(TaskState::Dying) {
                continue;
            }
            let resources = tasks.resources_mut(id).unwrap();
            if id == target {
                assert_eq!(resources.exit_code, 33);
                check_progress(resources);
                target_reaped = true;
            } else {
                assert_eq!(resources.exit_code, 7);
                caller_reaped = true;
            }
            drop(
                tasks
                    .reap(id, |resources| resources.close(pool, region))
                    .unwrap(),
            );
        }
    }
    assert!(target_reaped && caller_reaped);
    assert_eq!(tasks.live_count(), 0);
    assert_eq!(calls.denied, 3);
    assert_eq!(calls.invalid, 1);
    assert_eq!(calls.yields, 1);
    assert_eq!(control.reference_count(), 1);
    drop(control);
    assert_eq!(objects.live_count(), 0);
    let _ = kryo_hal::print(format_args!(
        "CAP: checks OK denied={} stale={}\n",
        calls.denied, calls.invalid
    ));
}

fn check_invalid_resume(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let mut tasks = Tasks::new();
    let bad = tasks
        .spawn(Resources::new(
            pool,
            direct,
            kernel,
            region,
            2,
            &programs::bad_stack(),
        ))
        .unwrap();
    let peer = tasks
        .spawn(Resources::new(
            pool,
            direct,
            kernel,
            region,
            7,
            &programs::counter(),
        ))
        .unwrap();
    let mut calls = CallStats::default();
    let mut ipc = IpcRuntime::new();
    let mut rejected = false;
    for _ in 0..8 {
        let (id, outcome) = step(&mut tasks, &mut calls, &mut ipc);
        if id == bad && outcome == Err(UserEntryError::InvalidAddress) {
            assert_eq!(tasks.state(id), Ok(TaskState::Dying));
            drop(
                tasks
                    .reap(id, |resources| resources.close(pool, region))
                    .unwrap(),
            );
            rejected = true;
        } else {
            assert_eq!(outcome.unwrap().reason, UserReason::Preempted);
        }
        if rejected && tasks.resources_mut(peer).unwrap().ticks >= 2 {
            break;
        }
    }
    assert!(rejected);
    check_progress(tasks.resources_mut(peer).unwrap());
    tasks.stop(peer).unwrap();
    drop(
        tasks
            .reap(peer, |resources| resources.close(pool, region))
            .unwrap(),
    );
    assert_eq!(tasks.live_count(), 0);
    kryo_hal::emergency_write_str("TASK: invalid saved state isolated\n");
}
