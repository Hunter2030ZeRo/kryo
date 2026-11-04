use super::{
    BootFramePool, CallStats, DirectMap, IpcRuntime, KernelMemory, KernelObject, KernelRegion,
    Objects, Program, RefCell, Resources, TaskState, Tasks, programs,
};
use crate::runtime::{KernelStack, ThreadResources, USER_STACKS, USER_TLS, smp, user_pages};
use kryo_core::capability::Rights;
use kryo_core::mm::frame::{FrameAllocError, OwnedFrame, PhysRange};
use kryo_core::mm::vm::Permissions;
use kryo_core::object::ObjectKind;
use kryo_core::process::{ExitReason, ExitStatus};
use kryo_core::task::TaskId;
use kryo_hal::{ExtendedState, UserContext};
use kryo_interfaces::syscall::Call;

const SECOND_ENTRY: u64 = programs::CODE + 512;
const TLB_SLOT: usize = 30;
const OLD_VALUE: u64 = 0x1234_5678_1234_5678;
const NEW_VALUE: u64 = 0x8765_4321_8765_4321;

#[derive(Clone, Copy)]
enum Case {
    Parallel,
    Wake,
    FaultCall,
}

#[derive(Clone, Copy)]
struct MigrationSample {
    data: [u64; 6],
    cpu_mask: kryo_hal::CpuSet,
    instruction: [u64; 2],
    ticks: [usize; 2],
}

pub(super) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
    cpus: usize,
) {
    check_remote_tlb(pool, direct, region, cpus);
    if cpus == 1 {
        kryo_hal::emergency_write_str("SMP: parallel user checks skipped cpus=1\n");
        return;
    }
    check_missing_ack(pool, direct, region, cpus);
    check_fault_call(pool, direct, memory, region, cpus);
    check_parallel(pool, direct, memory, region);
    kryo_hal::emergency_write_str("SMP: parallel shared-space OK\n");
    check_wakeup(pool, direct, memory, region);
}

fn probe_frame(pool: &BootFramePool) -> OwnedFrame<'_> {
    // Exercise higher physical-address bits whenever the RAM inventory permits.
    for minimum in [1 << 37, 1 << 32] {
        let span = PhysRange::from_base_len(minimum, (1 << 52) - minimum).unwrap();
        match pool.allocate_in(span) {
            Ok(frame) => return frame,
            Err(FrameAllocError::OutOfMemory) => {}
            Err(error) => panic!("Cannot allocate TLB probe backing: {:?}", error),
        }
    }
    pool.allocate().expect("Cannot allocate TLB probe backing")
}

fn check_remote_tlb(
    pool: &BootFramePool,
    direct: DirectMap,
    region: &mut KernelRegion,
    cpus: usize,
) {
    let before = pool.stats();
    let old = probe_frame(pool);
    let new = probe_frame(pool);
    let _ = kryo_hal::print(format_args!(
        "SMP: TLB backing old={:#x} new={:#x}\n",
        old.address(),
        new.address()
    ));
    unsafe {
        direct.zero_frame(old.address()).unwrap();
        direct.zero_frame(new.address()).unwrap();
        direct
            .write_frame(old.address(), 0, &OLD_VALUE.to_le_bytes())
            .unwrap();
        direct
            .write_frame(new.address(), 0, &NEW_VALUE.to_le_bytes())
            .unwrap();
        let address = region.map_ram(TLB_SLOT, old.address()).unwrap();
        for cpu in 0..cpus {
            assert_eq!(smp::probe_kernel(cpu, address), OLD_VALUE);
        }
        let prior_epoch = kryo_hal::tlb_epoch();
        assert_eq!(region.unmap(TLB_SLOT).unwrap(), old.address());
        assert_eq!(region.map_ram(TLB_SLOT, new.address()).unwrap(), address);
        let epoch = kryo_hal::tlb_epoch();
        assert!(epoch > prior_epoch);
        for cpu in 0..cpus {
            assert_eq!(kryo_hal::cpu_tlb_epoch(cpu), Ok(epoch));
            assert_eq!(smp::probe_kernel(cpu, address), NEW_VALUE);
        }
        assert_eq!(region.unmap(TLB_SLOT).unwrap(), new.address());
        // Both aliases and every remote translation retired before either
        // physical frame returns to the pool.
        pool.release(old).unwrap();
        pool.release(new).unwrap();
    }
    assert_eq!(pool.stats(), before);
    kryo_hal::emergency_write_str("SMP: remote TLB retirement OK\n");
}

fn check_missing_ack(
    pool: &'static BootFramePool,
    direct: DirectMap,
    region: &mut KernelRegion,
    cpus: usize,
) {
    let before = pool.stats();
    let old = probe_frame(pool);
    let replacement = probe_frame(pool);
    let allocated = pool.stats();
    let missing = cpus - 1;
    unsafe {
        direct.zero_frame(old.address()).unwrap();
        direct.zero_frame(replacement.address()).unwrap();
        direct
            .write_frame(old.address(), 0, &OLD_VALUE.to_le_bytes())
            .unwrap();
        direct
            .write_frame(replacement.address(), 0, &NEW_VALUE.to_le_bytes())
            .unwrap();
        let address = region.map_ram(TLB_SLOT, old.address()).unwrap();
        for cpu in 0..cpus {
            assert_eq!(smp::probe_kernel(cpu, address), OLD_VALUE);
        }
        kryo_hal::inject_tlb_miss(missing).unwrap();
        assert_eq!(
            region.unmap(TLB_SLOT),
            Err(kryo_hal::PagingError::ShootdownPending)
        );
        let pending_epoch = kryo_hal::tlb_epoch();
        assert!(kryo_hal::cpu_tlb_epoch(missing).unwrap() < pending_epoch);
        assert_eq!(
            region.permissions(TLB_SLOT),
            Err(kryo_hal::PagingError::ShootdownPending)
        );
        assert_eq!(
            region.map_ram(TLB_SLOT, replacement.address()),
            Err(kryo_hal::PagingError::ShootdownPending)
        );
        assert_eq!(kryo_hal::begin_user_lease(0), Err(kryo_hal::SmpError::Busy));
        assert_eq!(kryo_hal::begin_user_lease(1), Err(kryo_hal::SmpError::Busy));
        assert!(kryo_hal::active_user_mask().is_empty());
        assert_eq!(pool.stats(), allocated);

        assert_eq!(region.unmap(TLB_SLOT), Ok(old.address()));
        let epoch = kryo_hal::tlb_epoch();
        assert!(epoch > pending_epoch);
        for cpu in 0..cpus {
            assert_eq!(kryo_hal::cpu_tlb_epoch(cpu), Ok(epoch));
        }
        assert_eq!(
            region.permissions(TLB_SLOT),
            Err(kryo_hal::PagingError::NotMapped)
        );
        assert_eq!(region.map_ram(TLB_SLOT, replacement.address()), Ok(address));
        for cpu in 0..cpus {
            assert_eq!(smp::probe_kernel(cpu, address), NEW_VALUE);
        }
        assert_eq!(region.unmap(TLB_SLOT), Ok(replacement.address()));
        pool.release(old).unwrap();
        pool.release(replacement).unwrap();

        // A map can fail after publishing its PTE. The stack constructor must
        // retire that alias before returning its unpublished frame.
        kryo_hal::inject_tlb_miss(missing).unwrap();
    }
    let failure = KernelStack::try_new(pool, direct, region, 32).unwrap_err();
    assert_eq!(
        failure.error,
        crate::runtime::stacks::StackError::Paging(kryo_hal::PagingError::ShootdownPending)
    );
    assert_eq!(failure.cleanup_error, None);
    assert!(failure.retained.is_none());
    for slot in 31..=35 {
        assert_eq!(
            region.permissions(slot),
            Err(kryo_hal::PagingError::NotMapped)
        );
    }
    assert!(kryo_hal::all_users_quiescent());
    assert_eq!(pool.stats(), before);
    kryo_hal::emergency_write_str("SMP: missing ACK retains backing OK\n");
}

fn check_parallel(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::new();
    let mut calls = CallStats::default();
    let ids = prepare_pair(Case::Parallel, pool, direct, memory, region, &mut tasks);
    let process = tasks.process_for(ids[0]).unwrap();
    let mut parallel = false;
    let mut migrated = false;
    let mut previous_cpus = [None; 2];
    let mut samples = [MigrationSample {
        data: [0; 6],
        cpu_mask: kryo_hal::CpuSet::empty(),
        instruction: [0; 2],
        ticks: [0; 2],
    }; 8];
    for sample in &mut samples {
        let owner = tasks.process_mut(process).unwrap();
        owner.space.unmap_page(programs::DATA).unwrap();
        owner
            .space
            .map_page(programs::DATA, Permissions::ReadWrite, &[])
            .unwrap();
        for task in ids {
            tasks.thread_mut(task).unwrap().context.rip = programs::CODE;
        }
        let summary = smp::step(&mut tasks, &mut calls, &mut ipc, region);
        assert_eq!(summary.completed, 2);
        assert_eq!(summary.cpu_mask.count_ones(), 2);
        assert!(summary.shared_root);
        assert!(kryo_hal::all_users_quiescent());
        let data = snapshot(&mut tasks, ids[0]);
        sample.data = data;
        sample.cpu_mask = summary.cpu_mask;
        for (index, task) in ids.into_iter().enumerate() {
            assert_eq!(tasks.state(task), Ok(TaskState::Runnable));
            let thread = tasks.thread_mut(task).unwrap();
            assert_eq!(thread.context.fs_base, USER_TLS[index]);
            assert!(thread.ticks > 0);
            sample.instruction[index] = thread.context.rip;
            sample.ticks[index] = thread.ticks;
            if data[index] != 0 {
                if let Some(previous) = previous_cpus[index] {
                    migrated |= previous != data[index + 4];
                }
                previous_cpus[index] = Some(data[index + 4]);
            }
        }
        if data[0] != 0 && data[1] != 0 {
            // Fresh per-batch barrier data prevents progress left by an older
            // quantum from masquerading as simultaneous shared-root execution.
            assert_eq!(data[2], 3);
            assert_ne!(data[4], data[5]);
            parallel = true;
        }
    }
    if !parallel || !migrated {
        for (batch, sample) in samples.into_iter().enumerate() {
            let _ = kryo_hal::print(format_args!(
                "SMP migration diagnostic batch={} cpus={:?} counts={:?} barrier={} cpus={:?} rip={:#x?} ticks={:?}\n",
                batch,
                sample.cpu_mask,
                &sample.data[..2],
                sample.data[2],
                &sample.data[4..],
                sample.instruction,
                sample.ticks,
            ));
        }
    }
    assert!(parallel, "No batch advanced both shared-root threads");
    assert!(migrated, "User CPU identity did not change across batches");
    assert_eq!(calls.yields, 0);
    retire_pair(&mut tasks, &mut ipc, ids, pool, region);
    assert_eq!(pool.stats(), before);
}

fn check_wakeup(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::new();
    let mut calls = CallStats::default();
    let ids = prepare_pair(Case::Wake, pool, direct, memory, region, &mut tasks);
    let mut blocked = false;
    let mut single = false;
    let mut resumed = false;
    let mut peer_before_sleep = 0;
    for _ in 0..64 {
        let summary = smp::step(&mut tasks, &mut calls, &mut ipc, region);
        let data = snapshot(&mut tasks, ids[0]);
        if tasks.state(ids[0]) == Ok(TaskState::Blocked) {
            if !blocked {
                peer_before_sleep = data[1];
            }
            blocked = true;
        }
        if blocked && summary.completed == 1 {
            single = true;
        }
        if blocked && data[2] == 1 && data[0] > 0 {
            assert!(data[1] > peer_before_sleep);
            resumed = true;
            break;
        }
        assert_ne!(tasks.state(ids[0]), Ok(TaskState::Dying));
        assert_ne!(tasks.state(ids[1]), Ok(TaskState::Dying));
    }
    assert!(
        blocked && single && resumed,
        "SMP sleep/wakeup did not hand off execution"
    );
    retire_pair(&mut tasks, &mut ipc, ids, pool, region);
    assert_eq!(pool.stats(), before);
    kryo_hal::emergency_write_str("SMP: blocked thread wakeup OK\n");
}

fn check_fault_call(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
    cpus: usize,
) {
    let mut observed = false;
    for attempt in 0..cpus * 2 {
        if fault_call_once(pool, direct, memory, region, attempt == 0) {
            observed = true;
            break;
        }
    }
    assert!(
        observed,
        "No batch captured a fatal exception beside a pending native call"
    );
    kryo_hal::emergency_write_str("SMP: fatal process policy precedes peer call OK\n");
}

fn fault_call_once(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
    diagnostic: bool,
) -> bool {
    let before = pool.stats();
    let retained_before = memory.borrow().retained_pages();
    let heap_before = memory.borrow().transient_heap_usage();
    let objects = Objects::new();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::with_objects(&objects);
    let mut calls = CallStats::default();
    let ids = prepare_pair(Case::FaultCall, pool, direct, memory, region, &mut tasks);
    let process = tasks.process_for(ids[0]).unwrap();
    let debug = objects
        .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
        .unwrap();
    let handle = tasks
        .process_mut(process)
        .unwrap()
        .handles
        .insert(debug, Rights::WRITE)
        .unwrap();
    tasks.thread_mut(ids[1]).unwrap().context.r15 = handle.as_raw();
    let summary = smp::step_full_batch(&mut tasks, &mut calls, &mut ipc, region);
    assert_eq!(summary.completed, 2);
    assert!(summary.shared_root);
    let faulted = tasks.state(ids[0]) == Ok(TaskState::Dying)
        && tasks.thread_mut(ids[0]).unwrap().exit_reason == ExitReason::Fault;
    let called =
        tasks.thread_mut(ids[1]).unwrap().context.rip == SECOND_ENTRY + fault_call_return();
    let observed = faulted && called;
    if !observed && diagnostic {
        for task in ids {
            let state = tasks.state(task).unwrap();
            let owner = tasks.resources_mut(task).unwrap();
            let mut code = [0; 8];
            owner
                .process
                .space
                .read_page(programs::CODE, 0, &mut code)
                .unwrap();
            let _ = kryo_hal::print(format_args!(
                "SMP fault capture: id={:#x} state={:?} reason={:?} rip={:#x} ticks={} code={:x?}\n",
                task.as_raw(),
                state,
                owner.exit_reason,
                owner.context.rip,
                owner.ticks,
                code
            ));
        }
    }
    if observed {
        assert_eq!(
            calls.last_debug, None,
            "Killed process executed its peer's native call"
        );
        assert_eq!(tasks.state(ids[1]), Ok(TaskState::Dying));
        assert_eq!(
            tasks.terminal(process),
            Some(ExitStatus {
                reason: ExitReason::Fault,
                code: 0x100 + 6
            })
        );
    }
    retire_pair(&mut tasks, &mut ipc, ids, pool, region);
    drop(ipc);
    assert_eq!(objects.live_count(), 0);
    drop(tasks);
    drop(objects);
    let retained_after = memory.borrow().retained_pages();
    assert_eq!(memory.borrow().transient_heap_usage(), heap_before);
    assert_eq!(pool.stats().total, before.total);
    assert_eq!(
        pool.stats().free + retained_after,
        before.free + retained_before,
    );
    if retained_before != retained_after {
        let _ = kryo_hal::print(format_args!(
            "SMP: fault check retained kernel storage before={} after={}\n",
            retained_before, retained_after
        ));
    }
    observed
}

#[inline(never)]
fn prepare_pair<'kernel, 'objects>(
    case: Case,
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    tasks: &mut Tasks<'kernel, 'objects>,
) -> [TaskId; 2] {
    let mut first = Resources::new(pool, direct, memory, region, 2, &program(case));
    first
        .process
        .space
        .map_page(USER_TLS[0], Permissions::ReadWrite, &[])
        .unwrap();
    for page in user_pages(1) {
        first
            .process
            .space
            .map_page(page, Permissions::ReadWrite, &[])
            .unwrap();
    }
    configure(&mut first.context, 0);
    first.extended = Some(ExtendedState::new().unwrap());
    let first = tasks.spawn(first).unwrap();
    let process = tasks.process_for(first).unwrap();
    let entry = if matches!(case, Case::Parallel) {
        programs::CODE
    } else {
        SECOND_ENTRY
    };
    let mut context = UserContext::new(entry, USER_STACKS[1] + 16_384).unwrap();
    configure(&mut context, 1);
    let second = ThreadResources {
        gate_loan: None,
        process_id: None,
        execution: None,
        context,
        stack: KernelStack::new(pool, direct, region, 7),
        ticks: 0,
        exit_code: u64::MAX,
        exit_reason: ExitReason::Exited,
        wait: None,
        receive_slots: None,
        received_wait: None,
        received_handles: [None; 2],
        pool,
        memory,
        extended: Some(ExtendedState::new().unwrap()),
        direct,
        user_slot: 1,
        user_stack_owned: true,
        thread_control: None,
        thread_charge: None,
    };
    [first, tasks.spawn_thread(process, second).unwrap()]
}

fn configure(context: &mut UserContext, index: usize) {
    context.set_fs_base(USER_TLS[index]).unwrap();
    context.r9 = 1 << index;
    context.r10 = programs::DATA + 32 + index as u64 * 8;
    context.r11 = programs::DATA + 16;
    context.r12 = programs::DATA + index as u64 * 8;
    context.r13 = 0x1234_5678 + index as u64;
}

fn snapshot(tasks: &mut Tasks<'_, '_>, task: TaskId) -> [u64; 6] {
    assert!(kryo_hal::all_users_quiescent());
    let mut bytes = [0; 48];
    tasks
        .resources_mut(task)
        .unwrap()
        .process
        .space
        .read_page(programs::DATA, 0, &mut bytes)
        .unwrap();
    core::array::from_fn(|index| {
        u64::from_le_bytes(bytes[index * 8..index * 8 + 8].try_into().unwrap())
    })
}

#[inline(never)]
fn retire_pair<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    ids: [TaskId; 2],
    pool: &BootFramePool,
    region: &mut KernelRegion,
) {
    let process = tasks.process_for(ids[0]).unwrap();
    tasks
        .stop_process(
            process,
            ExitStatus {
                reason: ExitReason::Stopped,
                code: 0,
            },
        )
        .unwrap();
    ipc.settle(tasks);
    for task in ids {
        drop(
            tasks
                .reap(task, ipc, |resources| resources.close(pool, region))
                .unwrap(),
        );
    }
    assert_eq!(tasks.live_count(), 0);
    assert_eq!(tasks.process_count(), 0);
    assert_eq!(ipc.waits.next_ready(), None);
    assert_eq!(ipc.waits.next_deadline(), None);
}

fn program(case: Case) -> Program {
    let mut first = match case {
        Case::Parallel => counter(true),
        Case::Wake => sleeper(),
        Case::FaultCall => fault(),
    }
    .finish();
    assert!(first.as_bytes().len() <= 512);
    while first.as_bytes().len() < 512 {
        first.emit(&[0x90]);
    }
    let second = if matches!(case, Case::FaultCall) {
        fault_call()
    } else {
        counter(false)
    };
    first.emit(second.finish().as_bytes());
    first
}

fn counter(barrier: bool) -> Program {
    let mut code = Program::new();
    code.emit(&[0x64, 0x4c, 0x89, 0x2c, 0x25, 0, 0, 0, 0]);
    if barrier {
        code.emit(&[0xf0, 0x4d, 0x09, 0x0b]);
        code.emit(&[0x49, 0x83, 0x3b, 0x03, 0x75, 0xfa]);
    }
    counter_loop(&mut code);
    code
}

fn counter_loop(code: &mut Program) {
    let start = code.as_bytes().len();
    code.emit(&[0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]);
    code.emit(&[0x4c, 0x39, 0xe8]);
    code.failure_jump();
    code.emit(&[
        0xb8, 1, 0, 0, 0, 0x0f, 0xa2, 0xc1, 0xeb, 0x18, 0x49, 0x89, 0x1a,
    ]);
    code.emit(&[0xf0, 0x49, 0xff, 0x04, 0x24]);
    code.emit(&[0xe9]);
    let displacement =
        i32::try_from(start as isize - (code.as_bytes().len() + 4) as isize).unwrap();
    code.emit(&displacement.to_le_bytes());
}

fn sleeper() -> Program {
    let mut code = Program::new();
    code.emit(&[0x64, 0x4c, 0x89, 0x2c, 0x25, 0, 0, 0, 0]);
    code.call(Call::ClockRead, 0, 0);
    code.expect_status(0);
    code.emit(&[0x48, 0x81, 0xc2]);
    code.emit(&20_000_000u32.to_le_bytes());
    code.emit(&[0x48, 0x89, 0xd7, 0xb8]);
    code.emit(&(Call::SleepUntil as u32).to_le_bytes());
    code.emit(&[0xcd, 0x80]);
    code.expect_status(0);
    code.emit(&[0x49, 0xc7, 0x03, 1, 0, 0, 0]);
    counter_loop(&mut code);
    code
}

fn fault() -> Program {
    let mut code = Program::new();
    // Capture independent hardware return causes before testing batch policy;
    // a peer-dependent spin would require simultaneous emulated execution.
    code.emit(&[0x0f, 0x0b]);
    code
}

fn fault_call() -> Program {
    let mut code = Program::new();
    code.immediate(&[0x48, 0xbb], programs::DATA);
    code.immediate(&[0x48, 0xbe], 97);
    code.immediate(&[0x49, 0xb8], 1);
    code.emit(&[0xb8]);
    code.emit(&(Call::DebugLog as u32).to_le_bytes());
    code.emit(&[0x4c, 0x89, 0xff]);
    code.emit(&[0x48, 0xc7, 0x03, 1, 0, 0, 0, 0xcd, 0x80]);
    code
}

fn fault_call_return() -> u64 {
    fault_call().as_bytes().len() as u64
}
