pub(crate) mod abi;
pub(crate) mod atomic_wait;
pub(crate) mod block_service;
pub(crate) mod budgets;
pub(crate) mod channel_probe;
mod channels;
pub(crate) mod clock;
pub(crate) mod devices;
mod diagnostic;
const _: () = assert!(diagnostic::LIMIT == kryo_interfaces::syscall::DEBUG_WRITE_MAX);
pub(crate) mod ipc_runtime;
pub(crate) mod launch;
mod native;
mod private_vm;
mod process;
pub(crate) mod region;
#[cfg(feature = "boot-tests")]
pub(crate) mod scale;
pub(crate) mod service;
pub(crate) mod smp;
pub(crate) mod stacks;
pub(crate) mod startup;
pub(crate) mod supervision;
mod tasks;
pub(crate) mod threads;
#[cfg(feature = "boot-tests")]
pub(crate) use threads::native_checks::run as check_native_threads;
pub(crate) mod transport;

use crate::boot::memory::BootFramePool;
use crate::boot::vm_backend::{KernelMemory, UserSpace};
use core::cell::RefCell;
pub(crate) use ipc_runtime::IpcRuntime;
use kryo_core::capability::{Handle, HandleReservation, HandleTable, TransferBatch};
use kryo_core::ipc::{Endpoint, Notification};
use kryo_core::mm::shared::{SharedMemory, SharedPage};
use kryo_core::object::{ObjectRef, ObjectTable};
use kryo_core::process::{ExitReason, ProcessExit, ProcessId};
use kryo_core::resource::{Budget, Charge};
use kryo_core::sched::ReturnReason;
use kryo_core::sync::wait::WaitId;
use kryo_core::task::TaskId;
use kryo_hal::{DirectMap, ExtendedState, UserContext, UserEntryError, UserOutcome, UserReason};
pub(crate) use region::KernelRegion;
pub(crate) use stacks::KernelStack;
#[cfg(feature = "boot-tests")]
pub(crate) use tasks::ResourcesView;
pub(crate) use tasks::Tasks;

pub(crate) const REGION_BASE: u64 = 0xffff_ff00_0000_0000;
pub(crate) const OBJECTS: usize = 8;
pub(crate) const HANDLES: usize = 8;
pub(crate) type Objects = ObjectTable<KernelObject, OBJECTS>;
pub(crate) type Object<'objects> = ObjectRef<'objects, KernelObject, OBJECTS>;
pub(crate) type Handles<'objects> = HandleTable<'objects, KernelObject, OBJECTS, HANDLES>;
pub(crate) type Memory = SharedMemory<'static, BootFramePool, 8>;
pub(crate) type Batch<'objects> = TransferBatch<'objects, KernelObject, OBJECTS>;

pub(crate) struct SharedLease<'objects> {
    pub(crate) _owner: Object<'objects>,
    pub(crate) physical: u64,
}

// The strong arena reference retains the entire immutable memory object. VM
// drops this lease only after removing its mapping and completing TLB work.
unsafe impl SharedPage for SharedLease<'_> {
    fn physical(&self) -> u64 {
        self.physical
    }
}

pub(crate) enum KernelObject {
    #[cfg_attr(
        not(feature = "boot-tests"),
        expect(dead_code, reason = "Native object creation is not exposed yet")
    )]
    TaskControl(TaskId),
    Endpoint(RefCell<Endpoint<2, 2>>),
    Notification(RefCell<Notification>),
    ChannelFactory(kryo_core::ipc::ChannelBudget),
    ResourceBudget(Budget),
    Memory(Memory),
    ProcessControl(RefCell<ProcessControl>),
    ThreadControl(RefCell<ProcessExit<0>>),
    Image(&'static [u8]),
    DebugConsole,
    Device(RefCell<devices::DeviceBinding>),
    Transport(ProcessId),
    LaunchManifest(RefCell<startup::LaunchManifest>),
    StartupRecord(alloc::vec::Vec<u64>),
}

pub(crate) struct ProcessControl {
    pub(crate) process: Option<ProcessId>,
    pub(crate) exit: ProcessExit<0>,
}

impl ProcessControl {
    pub(crate) const fn new() -> Self {
        Self {
            process: None,
            exit: ProcessExit::growing(),
        }
    }
}

impl core::ops::Deref for ProcessControl {
    type Target = ProcessExit<0>;
    fn deref(&self) -> &Self::Target {
        &self.exit
    }
}

impl core::ops::DerefMut for ProcessControl {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.exit
    }
}

pub(crate) fn with_exit<R>(
    object: &KernelObject,
    operation: impl FnOnce(&mut ProcessExit<0>) -> R,
) -> Option<R> {
    match object {
        KernelObject::ProcessControl(control) => Some(operation(&mut control.borrow_mut().exit)),
        KernelObject::ThreadControl(control) => Some(operation(&mut control.borrow_mut())),
        _ => None,
    }
}

pub(crate) const USER_STACKS: [u64; 2] = [0x80_0000, 0x82_0000];
pub(crate) const USER_TLS: [u64; 2] = [0x90_0000, 0x92_0000];
const THREAD_ARENA: u64 = 0x4000_0000_0000;
const THREAD_STRIDE: u64 = 32 * 1024;

pub(crate) fn thread_addresses(slot: usize) -> Option<(u64, u64)> {
    if slot < USER_STACKS.len() {
        return Some((USER_STACKS[slot], USER_TLS[slot]));
    }
    let base = THREAD_ARENA.checked_add((slot as u64 - 2).checked_mul(THREAD_STRIDE)?)?;
    let stack = base.checked_add(4096)?;
    let tls = base.checked_add(24 * 1024)?;
    (tls.checked_add(4096)? < 1 << 47).then_some((stack, tls))
}

pub(crate) fn user_pages(slot: usize) -> [u64; 5] {
    let (stack, tls) = thread_addresses(slot).expect("Allocated user slot address invariant");
    [stack, stack + 4096, stack + 8192, stack + 12288, tls]
}

pub(crate) struct ProcessResources<'kernel, 'objects> {
    pub(crate) private_vm: kryo_core::mm::vm::PrivateVm,
    pub(crate) account: Option<Budget>,
    pub(crate) space: UserSpace<'kernel, SharedLease<'objects>>,
    pub(crate) handles: Handles<'objects>,
    pub(crate) control: Option<Object<'objects>>,
    pub(crate) pending: Option<launch::PendingLoad<'kernel, 'objects>>,
    pub(crate) pending_thread: Option<threads::PendingThread>,
    pub(crate) space_closed: bool,
}

pub(crate) struct ThreadResources<'kernel, 'objects> {
    // Mirrors process membership from attachment through successful retirement.
    pub(crate) process_id: Option<ProcessId>,
    pub(crate) execution: Option<kryo_core::mm::vm::ExecutionLease>,
    pub(crate) context: UserContext,
    pub(crate) stack: KernelStack,
    pub(crate) ticks: usize,
    pub(crate) exit_code: u64,
    pub(crate) exit_reason: ExitReason,
    pub(crate) wait: Option<WaitId>,
    pub(crate) receive_slots: Option<HandleReservation>,
    pub(crate) received_wait: Option<WaitId>,
    pub(crate) received_handles: [Option<Handle>; 2],
    pub(crate) pool: &'static BootFramePool,
    pub(crate) memory: &'kernel RefCell<KernelMemory>,
    pub(crate) extended: Option<ExtendedState>,
    pub(crate) direct: DirectMap,
    pub(crate) user_slot: usize,
    pub(crate) user_stack_owned: bool,
    pub(crate) thread_control: Option<Object<'objects>>,
    pub(crate) thread_charge: Option<Charge>,
}

pub(crate) struct Resources<'kernel, 'objects> {
    pub(crate) execution: Option<kryo_core::mm::vm::ExecutionLease>,
    pub(crate) process: ProcessResources<'kernel, 'objects>,
    pub(crate) context: UserContext,
    pub(crate) stack: KernelStack,
    pub(crate) ticks: usize,
    pub(crate) exit_code: u64,
    pub(crate) exit_reason: ExitReason,
    pub(crate) wait: Option<WaitId>,
    pub(crate) receive_slots: Option<HandleReservation>,
    pub(crate) received_wait: Option<WaitId>,
    pub(crate) received_handles: [Option<Handle>; 2],
    pub(crate) pool: &'static BootFramePool,
    pub(crate) memory: &'kernel RefCell<KernelMemory>,
    pub(crate) extended: Option<ExtendedState>,
    pub(crate) direct: DirectMap,
    pub(crate) user_slot: usize,
    pub(crate) user_stack_owned: bool,
    pub(crate) thread_control: Option<Object<'objects>>,
    pub(crate) thread_charge: Option<Charge>,
}

#[derive(Default)]
pub(crate) struct CallStats {
    #[cfg(feature = "boot-tests")]
    pub(crate) expired_receive_admissions: usize,
    #[cfg(feature = "boot-tests")]
    pub(crate) last_failure: Option<(
        TaskId,
        kryo_interfaces::syscall::Call,
        kryo_interfaces::syscall::Error,
    )>,
    pub(crate) yields: usize,
    pub(crate) denied: usize,
    pub(crate) invalid: usize,
    pub(crate) tls_sets: usize,
    pub(crate) spawned: usize,
    pub(crate) threads_created: usize,
    pub(crate) last_debug: Option<(TaskId, u64, u64)>,
}

impl ProcessResources<'_, '_> {
    pub(crate) fn close(
        &mut self,
        pool: &BootFramePool,
        region: &mut KernelRegion,
    ) -> Result<(), &'static str> {
        if let Some(pending) = self.pending_thread.as_mut() {
            if !pending.close(pool, region, &mut self.space) {
                return Err("Cannot retire unpublished thread");
            }
            self.pending_thread = None;
        }
        if let Some(pending) = self.pending.as_mut() {
            if !pending.close(pool, region) {
                return Err("Cannot retire unpublished process");
            }
            self.pending = None;
        }
        if !self.space_closed {
            self.space
                .close()
                .map_err(|_| "Cannot close process address space")?;
            self.space_closed = true;
        }
        self.private_vm
            .clear_after_close(&self.space)
            .map_err(|_| "Cannot retire private VM account")?;
        Ok(())
    }
}

impl ThreadResources<'_, '_> {
    pub(crate) fn close(
        &mut self,
        pool: &BootFramePool,
        region: &mut KernelRegion,
        space: &mut UserSpace<'_, SharedLease<'_>>,
    ) -> Result<(), &'static str> {
        if self.execution.is_some() {
            return Err("Thread execution memory has not retired");
        }
        unsafe { self.stack.close(pool, region) }.map_err(|_| "Cannot close thread entry stack")?;
        if self.user_stack_owned {
            for page in user_pages(self.user_slot) {
                match space.unmap_page(page) {
                    Ok(()) | Err(kryo_core::mm::vm::VmError::NotMapped) => {}
                    Err(_) => return Err("Cannot close thread user stack"),
                }
            }
            self.user_stack_owned = false;
        }
        Ok(())
    }
}

impl<'kernel, 'objects> Resources<'kernel, 'objects> {
    pub(crate) fn into_parts(
        self,
    ) -> (
        ProcessResources<'kernel, 'objects>,
        ThreadResources<'kernel, 'objects>,
    ) {
        (
            self.process,
            ThreadResources {
                process_id: None,
                execution: self.execution,
                context: self.context,
                stack: self.stack,
                ticks: self.ticks,
                exit_code: self.exit_code,
                exit_reason: self.exit_reason,
                wait: self.wait,
                receive_slots: self.receive_slots,
                received_wait: self.received_wait,
                received_handles: self.received_handles,
                pool: self.pool,
                memory: self.memory,
                extended: self.extended,
                direct: self.direct,
                user_slot: self.user_slot,
                user_stack_owned: self.user_stack_owned,
                thread_control: self.thread_control,
                thread_charge: self.thread_charge,
            },
        )
    }

    pub(crate) fn from_parts(
        process: ProcessResources<'kernel, 'objects>,
        thread: ThreadResources<'kernel, 'objects>,
    ) -> Self {
        Self {
            process,
            execution: thread.execution,
            context: thread.context,
            stack: thread.stack,
            ticks: thread.ticks,
            exit_code: thread.exit_code,
            exit_reason: thread.exit_reason,
            wait: thread.wait,
            receive_slots: thread.receive_slots,
            received_wait: thread.received_wait,
            received_handles: thread.received_handles,
            pool: thread.pool,
            memory: thread.memory,
            extended: thread.extended,
            direct: thread.direct,
            user_slot: thread.user_slot,
            user_stack_owned: thread.user_stack_owned,
            thread_control: thread.thread_control,
            thread_charge: thread.thread_charge,
        }
    }

    #[cfg(feature = "boot-tests")]
    pub(crate) fn close(
        &mut self,
        pool: &BootFramePool,
        region: &mut KernelRegion,
    ) -> Result<(), &'static str> {
        if let Some(lease) = self.execution.as_ref() {
            self.process
                .space
                .release_execution(lease)
                .map_err(|_| "Cannot release unpublished execution memory")?;
            self.execution = None;
        }
        unsafe { self.stack.close(pool, region) }
            .map_err(|_| "Cannot close unpublished entry stack")?;
        self.process.close(pool, region)
    }
}

#[cfg(feature = "boot-tests")]
pub(crate) fn step<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    calls: &mut CallStats,
    ipc: &mut IpcRuntime<'objects>,
) -> (TaskId, Result<UserOutcome, UserEntryError>) {
    step_inner(tasks, calls, ipc, None)
}

pub(crate) fn step_with_region<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    calls: &mut CallStats,
    ipc: &mut IpcRuntime<'objects>,
    region: &mut KernelRegion,
) -> (TaskId, Result<UserOutcome, UserEntryError>) {
    region
        .retry_extension()
        .expect("Shared stack branch retirement is still pending");
    step_inner(tasks, calls, ipc, Some(region))
}

fn step_inner<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    calls: &mut CallStats,
    ipc: &mut IpcRuntime<'objects>,
    region: Option<&mut KernelRegion>,
) -> (TaskId, Result<UserOutcome, UserEntryError>) {
    expire_waits(tasks, ipc);
    let now = kryo_hal::monotonic_time().expect("Missing scheduler clock");
    let quantum = now.saturating_add(2_000_000);
    let deadline = ipc
        .waits
        .next_deadline()
        .map_or(quantum, |next| next.min(quantum));
    let id = tasks
        .dispatch()
        .expect("Scheduler already owns a running task")
        .expect("No runnable task");
    let mut resources = tasks.resources_mut(id).expect("Missing running task");
    let top = resources.thread.stack.top();
    let context = &mut resources.thread.context;
    // The task borrow pins its private user root and guarded entry stack. The
    // HAL restores the boot root and saves all registers before this returns.
    let extended = &mut resources.thread.extended;
    let outcome = resources
        .process
        .space
        .with_root(|root| unsafe {
            kryo_hal::run_context_until(root, context, extended.as_mut(), top, deadline)
        })
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
            resources.exit_reason = ExitReason::Fault;
            ReturnReason::Fault
        }
        Ok(UserOutcome {
            reason: UserReason::Exited,
            status,
            ..
        }) => {
            resources.exit_code = status;
            resources.exit_reason = ExitReason::Exited;
            ReturnReason::Exit
        }
        Err(error) => {
            let _ = kryo_hal::print(format_args!("TASK rejected: {:?}\n", error));
            resources.exit_code = 255;
            resources.exit_reason = ExitReason::Fault;
            ReturnReason::Fault
        }
    };
    tasks.returned(id, reason).expect("Invalid task return");
    expire_waits(tasks, ipc);
    if matches!(
        outcome,
        Ok(UserOutcome {
            reason: UserReason::Call,
            ..
        })
    ) {
        native::dispatch(id, tasks, calls, ipc, region);
    }
    ipc.settle(tasks);
    (id, outcome)
}

pub(crate) fn expire_waits<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
) {
    ipc.expire(kryo_hal::monotonic_time().expect("Cannot read scheduler clock"));
    devices::poll(ipc);
    ipc.settle(tasks);
}

pub(crate) fn idle_for_events<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
) {
    expire_waits(tasks, ipc);
    if tasks.has_runnable() {
        return;
    }
    if ipc.waits.next_deadline().is_some() {
        idle_until_deadline(tasks, ipc);
    } else {
        assert!(
            devices::has_wake_source(ipc) || !ipc.words.is_empty(),
            "No registered wake source"
        );
        kryo_hal::idle_event().expect("Cannot idle for device IRQ");
        expire_waits(tasks, ipc);
    }
}

pub(crate) fn idle_until_deadline<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
) -> bool {
    expire_waits(tasks, ipc);
    if tasks.has_runnable() {
        return false;
    }
    let deadline = ipc
        .waits
        .next_deadline()
        .expect("No event can wake blocked tasks");
    // The run queue and deadlines were checked with IRQs off. No locks are
    // held across HLT; an early/stale IRQ only causes another clock check.
    unsafe { kryo_hal::arm_deadline(deadline) }.expect("Cannot arm idle deadline");
    kryo_hal::idle_until_interrupt().expect("Cannot halt for deadline");
    unsafe { kryo_hal::stop_timer() }.expect("Cannot stop idle event");
    expire_waits(tasks, ipc);
    true
}
