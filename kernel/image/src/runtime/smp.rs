use super::{CallStats, IpcRuntime, KernelRegion, Tasks};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use kryo_core::process::ExitReason;
use kryo_core::sched::{ReturnReason, TaskState};
use kryo_core::task::TaskId;
use kryo_hal::{CpuSet, ExtendedState, UserContext, UserEntryError, UserOutcome, UserReason};
use limine::mp::MpInfo;
use x86_64::registers::control::{Cr0, Cr3, Cr3Flags, Cr4};
use x86_64::registers::model_specific::Efer;
use x86_64::{PhysAddr, structures::paging::PhysFrame};

const IDLE: u8 = 0;
const READY: u8 = 1;
const RUNNING: u8 = 2;
const DONE: u8 = 3;
const STOP: u8 = 4;
const OFFLINE: u8 = 5;
const STARTING: u8 = 6;
const PARKED: u8 = 9;
#[cfg(feature = "boot-tests")]
const PROBE: u8 = 7;
#[cfg(feature = "boot-tests")]
const PROBE_DONE: u8 = 8;
#[cfg(feature = "boot-tests")]
const HEAP_PROBE: u8 = 10;
#[cfg(feature = "boot-tests")]
const HEAP_DONE: u8 = 11;
const WAIT_LIMIT_NS: u64 = 500_000_000;

struct Job {
    batch: u64,
    root: u64,
    context: UserContext,
    extended: Option<ExtendedState>,
    stack: u64,
    deadline: u64,
    outcome: Option<Result<UserOutcome, UserEntryError>>,
}

struct Mailbox {
    state: AtomicU8,
    notified: AtomicU64,
    job: UnsafeCell<Option<Job>>,
    #[cfg(feature = "boot-tests")]
    probe_address: AtomicU64,
    #[cfg(feature = "boot-tests")]
    probe_value: AtomicU64,
}

// READY transfers the owned payload to its CPU. DONE transfers it back to the
// coordinator. No worker receives a reference into Tasks or process resources.
unsafe impl Sync for Mailbox {}

impl Mailbox {
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(STARTING),
            notified: AtomicU64::new(0),
            job: UnsafeCell::new(None),
            #[cfg(feature = "boot-tests")]
            probe_address: AtomicU64::new(0),
            #[cfg(feature = "boot-tests")]
            probe_value: AtomicU64::new(0),
        }
    }
}

static MAILBOXES: [Mailbox; kryo_hal::MAX_CPUS] = [const { Mailbox::new() }; kryo_hal::MAX_CPUS];
static BOOT_ROOT: AtomicU64 = AtomicU64::new(0);
static BOOT_CR0: AtomicU64 = AtomicU64::new(0);
static BOOT_CR4: AtomicU64 = AtomicU64::new(0);
static BOOT_EFER: AtomicU64 = AtomicU64::new(0);
static APIC_MAPPING: AtomicU64 = AtomicU64::new(0);
static APIC_PHYSICAL: AtomicU64 = AtomicU64::new(0);
static CPUS: AtomicUsize = AtomicUsize::new(1);
static PREPARED: AtomicUsize = AtomicUsize::new(0);
static NEXT_CPU: AtomicUsize = AtomicUsize::new(1);
static NEXT_BATCH: AtomicU64 = AtomicU64::new(0);
static RELEASED_BATCH: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "boot-tests")]
static HEAP_ARRIVED: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "boot-tests")]
static HEAP_PHASE: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "boot-tests")]
static HEAP_DEADLINE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
pub(crate) struct BatchSummary {
    pub(crate) completed: usize,
    pub(crate) cpu_mask: CpuSet,
    pub(crate) shared_root: bool,
}

pub(crate) fn cpu_count() -> usize {
    CPUS.load(Ordering::Acquire)
}

pub(crate) fn prepare() {
    assert_eq!(kryo_hal::current_cpu().unwrap().index(), 0);
    let response = crate::boot::MP
        .response()
        .expect("Missing Limine MP response");
    assert!(
        !response.cpus().is_empty() && response.cpus().len() <= kryo_hal::MAX_CPUS,
        "Unsupported CPU count"
    );
    assert_eq!(response.flags & 1, 0, "x2APIC is not supported");
    assert_eq!(
        response.bsp_lapic_id,
        kryo_hal::cpu_apic_id(kryo_hal::current_cpu().unwrap())
    );
    assert_eq!(PREPARED.load(Ordering::Relaxed), 0, "APs already prepared");
    APIC_MAPPING.store(crate::boot::platform::apic_mapping(), Ordering::Relaxed);
    APIC_PHYSICAL.store(
        kryo_hal::timer_physical_address().expect("Missing BSP APIC"),
        Ordering::Relaxed,
    );
    let mut count = 1;
    for info in response.cpus() {
        if info.lapic_id == response.bsp_lapic_id || count == kryo_hal::MAX_CPUS {
            continue;
        }
        unsafe { kryo_hal::register_cpu(count, info.lapic_id) }.expect("Unsupported AP identity");
        // Limine retains AP stacks and acquires this bootstrap publication. Its
        // reclaimable pages remain excluded from the frame pool.
        info.bootstrap(ap_entry, count as u64);
        let mut acknowledged = false;
        for _ in 0..20_000_000 {
            if MAILBOXES[count].state.load(Ordering::Acquire) == PARKED {
                acknowledged = true;
                break;
            }
            core::hint::spin_loop();
        }
        assert!(acknowledged, "AP did not leave bootloader trampoline");
        count += 1;
    }
    PREPARED.store(count, Ordering::Release);
}

pub(crate) fn start(apic_mapping: u64) -> usize {
    assert_eq!(BOOT_ROOT.load(Ordering::Relaxed), 0, "SMP already started");
    let count = PREPARED.load(Ordering::Acquire);
    assert!(count > 0, "AP preparation was skipped");
    BOOT_CR0.store(Cr0::read_raw(), Ordering::Relaxed);
    BOOT_CR4.store(Cr4::read_raw(), Ordering::Relaxed);
    BOOT_EFER.store(Efer::read_raw(), Ordering::Relaxed);
    assert_eq!(APIC_MAPPING.load(Ordering::Acquire), apic_mapping);
    MAILBOXES[0].state.store(IDLE, Ordering::Release);
    unsafe { kryo_hal::set_cpu_online(0, true) }.expect("Cannot publish BSP");
    BOOT_ROOT.store(Cr3::read().0.start_address().as_u64(), Ordering::Release);
    for cpu in 1..count {
        match unsafe { kryo_hal::wake_boot_cpu(cpu) } {
            Ok(()) => {}
            Err(kryo_hal::SmpError::Busy) if kryo_hal::cpu_online_mask().contains(cpu) => {}
            Err(error) => panic!("Cannot wake prepared AP: {:?}", error),
        }
    }
    for mailbox in MAILBOXES.iter().take(count).skip(1) {
        wait_until(|| mailbox.state.load(Ordering::Acquire) == IDLE);
    }
    CPUS.store(count, Ordering::Release);
    let response = crate::boot::MP.response().unwrap();
    let _ = kryo_hal::print(format_args!(
        "SMP: online={} discovered={} limit={}\n",
        count,
        response.cpus().len(),
        kryo_hal::MAX_CPUS
    ));
    #[cfg(feature = "boot-tests")]
    check_heap(count);
    count
}

unsafe extern "C" fn ap_entry(info: &MpInfo) -> ! {
    unsafe { core::arch::asm!("cli", "cld", options(nostack)) };
    kryo_hal::validate_cpu_baseline().expect("x86-64-v3 hardware required");
    let cpu = info.extra_argument() as usize;
    assert!(cpu > 0 && cpu < kryo_hal::MAX_CPUS);
    kryo_hal::init_gdt();
    kryo_hal::init_idt();
    assert_eq!(kryo_hal::current_cpu().unwrap().index(), cpu);
    let _ = kryo_hal::print(format_args!(
        "CPU: x86-64-v3 hardware admitted cpu={}\n",
        cpu
    ));
    unsafe {
        Efer::write_raw(Efer::read_raw() | (1 << 11));
        Cr3::write(
            PhysFrame::from_start_address(PhysAddr::new(crate::boot::platform::root_frame()))
                .unwrap(),
            Cr3Flags::empty(),
        );
    }
    assert_eq!(
        kryo_hal::timer_physical_address().expect("Unsupported AP controller"),
        APIC_PHYSICAL.load(Ordering::Acquire)
    );
    unsafe { kryo_hal::init_timer(APIC_MAPPING.load(Ordering::Acquire)) }
        .expect("Cannot initialize early AP controller");
    // Leave Limine's HHDM trampoline before NX policy, then sleep on the retained
    // supervisor APIC mapping until the BSP publishes final protection state.
    MAILBOXES[cpu].state.store(PARKED, Ordering::Release);
    while BOOT_ROOT.load(Ordering::Acquire) == 0 {
        kryo_hal::idle_event().expect("Cannot idle prepared AP");
    }
    // Establish the BSP's protection state before using its current page tables.
    unsafe {
        Efer::write_raw(BOOT_EFER.load(Ordering::Acquire));
        Cr4::write_raw(BOOT_CR4.load(Ordering::Acquire));
        Cr0::write_raw(BOOT_CR0.load(Ordering::Acquire));
        Cr3::write(
            PhysFrame::from_start_address(PhysAddr::new(BOOT_ROOT.load(Ordering::Acquire)))
                .unwrap(),
            Cr3Flags::empty(),
        );
    }
    assert_eq!(
        kryo_hal::timer_physical_address().expect("Unsupported AP controller"),
        APIC_PHYSICAL.load(Ordering::Acquire),
        "APIC physical bases differ"
    );
    unsafe { kryo_hal::calibrate_timer() }.expect("Cannot calibrate AP timer");
    unsafe { kryo_hal::set_cpu_online(cpu, true) }.expect("Cannot publish AP");
    let mailbox = &MAILBOXES[cpu];
    mailbox.state.store(IDLE, Ordering::Release);
    notify(0);
    loop {
        match mailbox.state.load(Ordering::Acquire) {
            #[cfg(feature = "boot-tests")]
            HEAP_PROBE => {
                heap_work(cpu, cpu_count());
                mailbox.state.store(HEAP_DONE, Ordering::Release);
                notify(0);
            }
            #[cfg(feature = "boot-tests")]
            PROBE => {
                let address = mailbox.probe_address.load(Ordering::Relaxed);
                let value = unsafe { (address as *const u64).read_volatile() };
                mailbox.probe_value.store(value, Ordering::Relaxed);
                mailbox.state.store(PROBE_DONE, Ordering::Release);
                notify(0);
            }
            READY => {
                mailbox.state.store(RUNNING, Ordering::Relaxed);
                let mut job = unsafe { (*mailbox.job.get()).take().expect("Missing CPU job") };
                execute(&mut job);
                let batch = job.batch;
                // run_context_until restores the boot root and saves all user
                // state before this release. Its stack and root stay retained.
                unsafe { *mailbox.job.get() = Some(job) };
                mailbox.state.store(DONE, Ordering::Release);
                notify(0);
                mailbox.notified.store(batch, Ordering::Release);
            }
            STOP => {
                unsafe { kryo_hal::shutdown_timer() }.expect("Cannot retire AP timer");
                unsafe { kryo_hal::set_cpu_online(cpu, false) }.expect("Cannot retire AP identity");
                mailbox.state.store(OFFLINE, Ordering::Release);
                // No MMIO, clock, or task resources are touched after OFFLINE.
                loop {
                    unsafe { core::arch::asm!("cli", "hlt", options(nostack)) }
                }
            }
            _ => {
                kryo_hal::idle_event().expect("Cannot idle AP");
            }
        }
    }
}

#[cfg(feature = "boot-tests")]
fn check_heap(count: usize) {
    let before = crate::heap::stats();
    HEAP_DEADLINE.store(
        kryo_hal::monotonic_time()
            .expect("Missing heap probe clock")
            .saturating_add(10_000_000_000),
        Ordering::Release,
    );
    for (cpu, mailbox) in MAILBOXES.iter().enumerate().take(count).skip(1) {
        assert_eq!(mailbox.state.load(Ordering::Acquire), IDLE);
        mailbox.state.store(HEAP_PROBE, Ordering::Release);
        notify(cpu);
    }
    heap_work(0, count);
    wait_until(|| {
        MAILBOXES[1..count]
            .iter()
            .all(|mailbox| mailbox.state.load(Ordering::Acquire) == HEAP_DONE)
    });
    assert_eq!(
        crate::heap::stats(),
        before,
        "Concurrent heap owners survived"
    );
    for mailbox in &MAILBOXES[1..count] {
        mailbox.state.store(IDLE, Ordering::Release);
    }
    let _ = kryo_hal::print(format_args!(
        "HEAP: concurrent allocation and retirement OK cpus={}\n",
        count
    ));
}

#[cfg(feature = "boot-tests")]
fn heap_barrier(count: usize) {
    let phase = HEAP_PHASE.load(Ordering::Acquire);
    if HEAP_ARRIVED.fetch_add(1, Ordering::AcqRel) + 1 == count {
        HEAP_ARRIVED.store(0, Ordering::Relaxed);
        HEAP_PHASE.store(phase + 1, Ordering::Release);
        return;
    }
    let mut spins = 0usize;
    while HEAP_PHASE.load(Ordering::Acquire) == phase {
        if spins.is_multiple_of(256) {
            assert!(
                kryo_hal::monotonic_time().unwrap() < HEAP_DEADLINE.load(Ordering::Acquire),
                "Concurrent heap phase stalled"
            );
        }
        spins = spins.wrapping_add(1);
        core::hint::spin_loop();
    }
}

#[cfg(feature = "boot-tests")]
fn heap_work(cpu: usize, count: usize) {
    use alloc::alloc::{alloc, dealloc};
    use core::alloc::Layout;

    heap_barrier(count);
    for round in 0..16 {
        let mut objects = [(core::ptr::null_mut::<u8>(), Layout::new::<u64>()); 16];
        for (slot, (pointer, layout)) in objects.iter_mut().enumerate() {
            let choice = (cpu + round + slot) % 4;
            *layout = Layout::from_size_align([8, 24, 64, 160][choice], [8, 16, 64, 128][choice])
                .unwrap();
            *pointer = unsafe { alloc(*layout) };
            assert!(!pointer.is_null(), "Concurrent heap allocation failed");
            assert_eq!(*pointer as usize % layout.align(), 0);
            let pattern = ((cpu as u64) << 48) | ((round as u64) << 32) | ((slot as u64) << 16);
            for word in 0..layout.size() / 8 {
                unsafe {
                    pointer
                        .cast::<u64>()
                        .add(word)
                        .write_volatile(pattern ^ word as u64)
                };
            }
        }
        // Every CPU retains its allocations through all peer writes and reads.
        // Distinct patterns expose aliasing without sharing Rust references.
        heap_barrier(count);
        for (slot, (pointer, layout)) in objects.iter().enumerate() {
            let pattern = ((cpu as u64) << 48) | ((round as u64) << 32) | ((slot as u64) << 16);
            for word in 0..layout.size() / 8 {
                assert_eq!(
                    unsafe { pointer.cast::<u64>().add(word).read_volatile() },
                    pattern ^ word as u64,
                    "Concurrent heap allocations overlap"
                );
            }
        }
        heap_barrier(count);
        for (pointer, layout) in objects {
            unsafe { dealloc(pointer, layout) };
        }
    }
}

/// Reads one retained supervisor mapping on an idle CPU for TLB validation.
/// The caller owns the mapping, excludes writers, and retains it until return.
#[cfg(feature = "boot-tests")]
pub(crate) unsafe fn probe_kernel(cpu: usize, address: u64) -> u64 {
    assert!(kryo_hal::all_users_quiescent() && cpu < cpu_count());
    if cpu == 0 {
        return unsafe { (address as *const u64).read_volatile() };
    }
    let mailbox = &MAILBOXES[cpu];
    assert_eq!(mailbox.state.load(Ordering::Acquire), IDLE);
    mailbox.probe_address.store(address, Ordering::Relaxed);
    mailbox.state.store(PROBE, Ordering::Release);
    notify(cpu);
    wait_until(|| mailbox.state.load(Ordering::Acquire) == PROBE_DONE);
    let value = mailbox.probe_value.load(Ordering::Relaxed);
    mailbox.state.store(IDLE, Ordering::Release);
    value
}

fn notify(cpu: usize) {
    kryo_hal::send_reschedule(cpu).expect("Cannot deliver CPU wakeup");
}

fn execute(job: &mut Job) {
    while RELEASED_BATCH.load(Ordering::Acquire) < job.batch {
        core::hint::spin_loop();
    }
    kryo_hal::drain_reschedule().expect("Cannot retire queued CPU wakeup");
    let quantum = kryo_hal::monotonic_time()
        .unwrap()
        .saturating_add(2_000_000);
    let deadline = job.deadline.min(quantum);
    job.outcome = Some(unsafe {
        kryo_hal::run_context_until(
            job.root,
            &mut job.context,
            job.extended.as_mut(),
            job.stack,
            deadline,
        )
    });
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = kryo_hal::monotonic_time()
        .expect("Missing SMP clock")
        .saturating_add(WAIT_LIMIT_NS);
    while !ready() {
        assert!(
            kryo_hal::monotonic_time().unwrap() < deadline,
            "CPU did not acknowledge; execution owners retained"
        );
        unsafe { kryo_hal::arm_deadline(deadline) }.expect("Cannot arm CPU wait watchdog");
        kryo_hal::idle_until_interrupt().expect("Cannot wait for CPU event");
    }
    unsafe { kryo_hal::stop_timer() }.expect("Cannot stop CPU wait watchdog");
}

pub(crate) fn step<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    calls: &mut CallStats,
    ipc: &mut IpcRuntime<'objects>,
    region: &mut KernelRegion,
) -> BatchSummary {
    step_inner(tasks, calls, ipc, region, true)
}

#[cfg(feature = "boot-tests")]
pub(crate) fn step_full_batch<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    calls: &mut CallStats,
    ipc: &mut IpcRuntime<'objects>,
    region: &mut KernelRegion,
) -> BatchSummary {
    // Fault/call ordering checks need both real entry causes in one captured
    // batch. Let each job finish its unchanged quantum without an early E1.
    step_inner(tasks, calls, ipc, region, false)
}

fn step_inner<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    calls: &mut CallStats,
    ipc: &mut IpcRuntime<'objects>,
    region: &mut KernelRegion,
    interrupt_peers: bool,
) -> BatchSummary {
    region
        .retry_extension()
        .expect("Shared stack branch retirement is still pending");
    if cpu_count() == 1 {
        let _ = super::step_with_region(tasks, calls, ipc, region);
        return BatchSummary {
            completed: 1,
            cpu_mask: CpuSet::singleton(0),
            shared_root: false,
        };
    }
    super::expire_waits(tasks, ipc);
    let deadline = ipc.waits.next_deadline().unwrap_or(u64::MAX);
    let batch = NEXT_BATCH
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .expect("Execution batch epoch exhausted")
        + 1;
    let mut assigned: [Option<TaskId>; kryo_hal::MAX_CPUS] = [None; kryo_hal::MAX_CPUS];
    let mut roots = [0; kryo_hal::MAX_CPUS];
    let mut summary = BatchSummary {
        completed: 0,
        cpu_mask: CpuSet::empty(),
        shared_root: false,
    };
    let workers = cpu_count();
    let first = NEXT_CPU.fetch_add(1, Ordering::Relaxed) % workers;
    // This exclusive Tasks borrow lasts across the whole batch. Root/stack
    // allocation owners stay in Running slots; only copied/moved CPU state leaves.
    for offset in 0..workers {
        let cpu = (first + offset) % workers;
        let Some(task) = tasks.dispatch_on(cpu).expect("CPU already owns a thread") else {
            break;
        };
        let resources = tasks.resources_mut(task).unwrap();
        let root = resources
            .process
            .space
            .root_frame()
            .expect("Closed running address space");
        let mailbox = &MAILBOXES[cpu];
        assert_eq!(mailbox.state.load(Ordering::Acquire), IDLE);
        let job = Job {
            batch,
            root,
            context: resources.thread.context,
            extended: resources.thread.extended.take(),
            stack: resources.thread.stack.top(),
            deadline,
            outcome: None,
        };
        unsafe { *mailbox.job.get() = Some(job) };
        unsafe { kryo_hal::begin_user_lease(cpu) }.expect("Duplicate CPU execution lease");
        roots[cpu] = root;
        assigned[cpu] = Some(task);
        summary.completed += 1;
        summary.cpu_mask.insert(cpu);
        mailbox.state.store(READY, Ordering::Release);
    }
    assert!(summary.completed > 0, "No runnable SMP thread");
    summary.shared_root = roots
        .iter()
        .enumerate()
        .any(|(cpu, root)| *root != 0 && roots[..cpu].contains(root));
    for (cpu, task) in assigned.iter().enumerate().skip(1) {
        if task.is_some() {
            notify(cpu);
        }
    }
    RELEASED_BATCH.store(batch, Ordering::Release);
    if assigned[0].is_some() {
        let mailbox = &MAILBOXES[0];
        mailbox.state.store(RUNNING, Ordering::Relaxed);
        let mut job = unsafe { (*mailbox.job.get()).take().unwrap() };
        execute(&mut job);
        unsafe { *mailbox.job.get() = Some(job) };
        mailbox.state.store(DONE, Ordering::Release);
    }
    if interrupt_peers {
        wait_until(|| {
            assigned.iter().enumerate().any(|(cpu, task)| {
                task.is_some() && MAILBOXES[cpu].state.load(Ordering::Acquire) == DONE
            })
        });
        for (cpu, task) in assigned.iter().enumerate() {
            if task.is_some() && MAILBOXES[cpu].state.load(Ordering::Acquire) != DONE {
                notify(cpu);
            }
        }
    }
    wait_until(|| {
        assigned.iter().enumerate().all(|(cpu, task)| {
            task.is_none() || MAILBOXES[cpu].state.load(Ordering::Acquire) == DONE
        })
    });
    let notification_deadline = kryo_hal::monotonic_time()
        .unwrap()
        .saturating_add(WAIT_LIMIT_NS);
    while assigned.iter().enumerate().skip(1).any(|(cpu, task)| {
        task.is_some() && MAILBOXES[cpu].notified.load(Ordering::Acquire) < batch
    }) {
        assert!(
            kryo_hal::monotonic_time().unwrap() < notification_deadline,
            "CPU completion notification did not retire"
        );
        core::hint::spin_loop();
    }
    let mut results = [None; kryo_hal::MAX_CPUS];
    let mut reasons = [ReturnReason::Preempted; kryo_hal::MAX_CPUS];
    for (cpu, task) in assigned.iter().enumerate() {
        let Some(task) = *task else { continue };
        let mailbox = &MAILBOXES[cpu];
        let job = unsafe {
            (*mailbox.job.get())
                .take()
                .expect("Missing completed CPU job")
        };
        let resources = tasks.thread_mut(task).unwrap();
        resources.context = job.context;
        resources.extended = job.extended;
        let outcome = job.outcome.expect("Missing CPU return");
        #[cfg(feature = "boot-tests")]
        if !interrupt_peers
            && let Ok(UserOutcome {
                reason: UserReason::Preempted,
                vector,
                ..
            }) = outcome
        {
            let _ = kryo_hal::print(format_args!(
                "SMP capture preempted: cpu={} vector={:#x}\n",
                cpu, vector
            ));
        }
        reasons[cpu] = match outcome {
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
                reason: UserReason::Exited,
                status,
                ..
            }) => {
                resources.exit_code = status;
                resources.exit_reason = ExitReason::Exited;
                ReturnReason::Exit
            }
            Ok(UserOutcome {
                reason: UserReason::Fault,
                vector,
                ..
            }) => {
                resources.exit_code = 0x100 + vector;
                resources.exit_reason = ExitReason::Fault;
                ReturnReason::Fault
            }
            Err(_) => {
                resources.exit_code = 255;
                resources.exit_reason = ExitReason::Fault;
                ReturnReason::Fault
            }
        };
        results[cpu] = Some(outcome);
        mailbox.state.store(IDLE, Ordering::Release);
        unsafe { kryo_hal::end_user_lease(cpu) }.expect("Missing CPU execution lease");
        tasks
            .returned_on(cpu, task, reasons[cpu])
            .expect("Invalid CPU return ownership");
    }
    assert!(kryo_hal::all_users_quiescent());
    // Every CPU ownership record ends before process-wide fault/exit policy.
    for (cpu, task) in assigned.iter().enumerate() {
        if let Some(task) = *task {
            tasks
                .apply_return_policy(task, reasons[cpu])
                .expect("Invalid process return policy");
        }
    }
    super::expire_waits(tasks, ipc);
    for (cpu, task) in assigned.iter().enumerate() {
        if let Some(task) = *task
            && matches!(
                results[cpu],
                Some(Ok(UserOutcome {
                    reason: UserReason::Call,
                    ..
                }))
            )
            && tasks.state(task) == Ok(TaskState::Blocked)
        {
            super::native::dispatch(task, tasks, calls, ipc, Some(region));
        }
    }
    ipc.settle(tasks);
    summary
}

#[cfg(feature = "boot-tests")]
pub(crate) fn shutdown() {
    assert!(kryo_hal::all_users_quiescent());
    let peers = &MAILBOXES[1..cpu_count()];
    for mailbox in peers {
        assert_eq!(mailbox.state.load(Ordering::Acquire), IDLE);
        mailbox.state.store(STOP, Ordering::Release);
    }
    for cpu in 1..cpu_count() {
        // An older wake can let this AP finish STOP before its new IPI.
        match kryo_hal::send_reschedule(cpu) {
            Ok(()) | Err(kryo_hal::SmpError::Offline) => {}
            Err(error) => panic!("Cannot deliver CPU shutdown: {:?}", error),
        }
    }
    // An offline AP cannot send an IPI after retiring its controller. One
    // watchdog bounds the complete handoff; shared owners remain retained.
    wait_until(|| {
        peers
            .iter()
            .all(|mailbox| mailbox.state.load(Ordering::Acquire) == OFFLINE)
    });
    CPUS.store(1, Ordering::Release);
    kryo_hal::emergency_write_str("SMP: AP resources retired\n");
}
