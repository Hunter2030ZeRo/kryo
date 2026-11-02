mod exec_checks;
mod ipc_checks;
mod ipc_programs;
mod launch_checks;
mod programs;
mod shared_checks;
mod shared_programs;
mod smp_checks;
mod spawn_checks;
mod thread_checks;
mod time_checks;
mod time_programs;

use super::memory::BootFramePool;
use super::vm_backend::{KernelMemory, new_shared_space};
use crate::runtime::KernelRegion;
use crate::runtime::{
    CallStats, Handles, IpcRuntime, KernelObject, KernelStack, Objects, ProcessResources,
    Resources, SharedLease, Tasks, clock, idle_until_deadline, step,
};
use core::cell::RefCell;
use kryo_core::capability::Rights;
use kryo_core::mm::vm::Permissions;
use kryo_core::object::ObjectKind;
use kryo_core::sched::TaskState;
use kryo_hal::{DirectMap, ExtendedState, UserContext, UserEntryError, UserReason};
use programs::{CODE, DATA, Program, SEED_A, SEED_B, STACK};

pub(super) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let retained_before = kernel.borrow().retained_pages();
    crate::boot::storage::check_buffers(pool, direct, kernel);
    let controller = kryo_hal::timer_physical_address().expect("No supported local APIC");
    let mmio = crate::boot::platform::apic_mapping();
    let flags = region.permissions(0).expect("Missing APIC mapping");
    assert!(flags.writable && flags.uncached && !flags.executable && !flags.user);
    let clock_physical = clock::initialize(region);

    check_preemption(pool, direct, kernel, region);
    check_capabilities(pool, direct, kernel, region);
    check_invalid_resume(pool, direct, kernel, region);
    ipc_checks::run(pool, direct, kernel, region);
    shared_checks::run(pool, direct, kernel, region);
    exec_checks::run(pool, direct, kernel, region);
    time_checks::run(pool, direct, kernel, region);
    launch_checks::run(pool, direct, kernel, region);
    spawn_checks::run(pool, direct, kernel, region);
    thread_checks::run(pool, direct, kernel, region);
    let cpus = crate::runtime::smp::start(mmio);
    crate::runtime::transport::run(pool, direct, kernel, region);
    crate::runtime::block_service::check(pool, direct, kernel, region);
    crate::runtime::devices::probe(pool, direct, kernel, region, true);
    crate::runtime::channel_probe::run(pool, direct, kernel, region);
    crate::runtime::supervision::check(pool, direct, kernel, region);
    smp_checks::run(pool, direct, kernel, region, cpus);
    crate::runtime::service::check(pool, direct, kernel, region);
    crate::runtime::scale::check(pool, direct, kernel, region);
    crate::runtime::smp::shutdown();

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
    clock::shutdown(region, clock_physical);
    // All task roots and entry-stack accesses are retired; IRQs remain off.
    unsafe {
        assert_eq!(region.unmap(0).expect("Cannot unmap APIC"), controller);
        region.detach().expect("Cannot detach kernel task region");
    }
    let retained_after = kernel.borrow().retained_pages();
    assert_eq!(pool.stats().total, before.total);
    assert_eq!(
        pool.stats().free,
        before.free - (retained_after - retained_before)
    );
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
        let mut space = new_shared_space(pool, kernel).expect("Cannot create task address space");
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
            process: ProcessResources {
                account: None,
                space,
                handles: Handles::new(),
                control: None,
                pending: None,
                pending_thread: None,
                space_closed: false,
            },
            context,
            stack: KernelStack::new(pool, direct, region, first_slot),
            ticks: 0,
            exit_code: u64::MAX,
            exit_reason: kryo_core::process::ExitReason::Exited,
            wait: None,
            receive_slots: None,
            received_wait: None,
            received_handles: [None; 2],
            pool,
            memory: kernel,
            extended: None,
            direct,
            user_slot: 0,
            user_stack_owned: false,
            thread_control: None,
            thread_charge: None,
        }
    }
}

fn check_progress(resources: &crate::runtime::ResourcesView<'_, '_, '_>) {
    assert_eq!(resources.context.r12, SEED_A);
    assert_eq!(resources.context.r13, SEED_B);
    let mut bytes = [0; 8];
    resources
        .process
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
        check_progress(&resources);
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
    controller.process.handles = handles;
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
                check_progress(&resources);
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
    check_progress(&tasks.resources_mut(peer).unwrap());
    tasks.stop(peer).unwrap();
    drop(
        tasks
            .reap(peer, |resources| resources.close(pool, region))
            .unwrap(),
    );
    assert_eq!(tasks.live_count(), 0);
    kryo_hal::emergency_write_str("TASK: invalid saved state isolated\n");
}
