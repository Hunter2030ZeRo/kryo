use super::{BootFramePool, CallStats, ExtendedState, IpcRuntime, KernelObject};
use super::{KernelRegion, KernelStack, Object, Tasks, ThreadResources};
use super::{UserContext, UserSpace, thread_addresses, user_pages};
use core::cell::RefCell;
use kryo_core::capability::{Rights, TransferBatch};
use kryo_core::ipc::{Completion, Delivery};
use kryo_core::mm::vm::{Permissions, VmError};
use kryo_core::object::ObjectKind;
use kryo_core::process::{ExitReason, ProcessExit};
use kryo_core::resource::{Charge, Resource};
use kryo_core::task::TaskId;
use kryo_interfaces::syscall::Error;

#[cfg(feature = "boot-tests")]
pub(crate) mod native_checks;

/// Resources of an unpublished thread remain owned until cleanup finishes.
pub(crate) struct PendingThread {
    stack: Option<KernelStack>,
    attempted: [bool; 5],
    user_slot: usize,
    thread_charge: Option<Charge>,
}

impl PendingThread {
    /// Used after process-wide invalidation or before these mappings are visible
    /// to user execution. Active-process retries must use close_with instead.
    pub(crate) fn close<S: kryo_core::mm::shared::SharedPage>(
        &mut self,
        pool: &BootFramePool,
        region: &mut KernelRegion,
        space: &mut UserSpace<'_, S>,
    ) -> bool {
        self.close_with(pool, region, space, |_, _| {})
    }

    pub(crate) fn close_with<S: kryo_core::mm::shared::SharedPage>(
        &mut self,
        pool: &BootFramePool,
        region: &mut KernelRegion,
        space: &mut UserSpace<'_, S>,
        mut before: impl FnMut(u64, u64),
    ) -> bool {
        if let Some(stack) = self.stack.as_mut() {
            // No CPU has entered this unpublished stack.
            if unsafe { stack.close(pool, region) }.is_err() {
                return false;
            }
            self.stack = None;
        }
        if self.attempted.iter().any(|attempted| *attempted) {
            for (index, page) in user_pages(self.user_slot).into_iter().enumerate() {
                if self.attempted[index] {
                    match space.unmap_page_with(page, || before(page, page + 4096)) {
                        Ok(()) | Err(VmError::NotMapped) => self.attempted[index] = false,
                        Err(_) => return false,
                    }
                }
            }
        }
        self.thread_charge = None;
        true
    }
}

pub(crate) fn reserved_address(address: u64) -> bool {
    address >= super::THREAD_ARENA
        || super::USER_STACKS
            .into_iter()
            .any(|stack| (stack - 4096..stack + 16_384).contains(&address))
        || super::USER_TLS
            .into_iter()
            .any(|tls| (tls..tls + 4096).contains(&address))
}

pub(super) fn create<'kernel, 'objects>(
    caller: TaskId,
    context: UserContext,
    tasks: &mut Tasks<'kernel, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    region: &mut KernelRegion,
) -> Result<u64, Error> {
    create_thread(
        caller,
        Start::Managed {
            entry: context.rdi,
            argument: context.rsi,
        },
        tasks,
        ipc,
        stats,
        region,
    )
}

pub(super) fn create_native<'kernel, 'objects>(
    caller: TaskId,
    context: UserContext,
    tasks: &mut Tasks<'kernel, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    region: &mut KernelRegion,
) -> Result<u64, Error> {
    create_thread(
        caller,
        Start::Native(NativeStart::new(context)?),
        tasks,
        ipc,
        stats,
        region,
    )
}

enum Start {
    Managed { entry: u64, argument: u64 },
    Native(NativeStart),
}

impl Start {
    fn entry(&self) -> u64 {
        match self {
            Self::Managed { entry, .. } => *entry,
            Self::Native(start) => start.entry,
        }
    }
}

struct NativeStart {
    entry: u64,
    argument: u64,
    stack_base: u64,
    stack_bytes: u64,
    stack_end: u64,
    tls_base: u64,
    tls_bytes: u64,
    fs_base: u64,
    guard_bytes: u64,
}

impl NativeStart {
    fn new(context: UserContext) -> Result<Self, Error> {
        if context.rdi != 1 || context.r14 != 0 {
            return Err(Error::InvalidArgument);
        }
        let stack_end = user_end(context.r8, context.r9)?;
        let tls_end = user_end(context.r10, context.r11)?;
        if context.r9 < 16
            || stack_end >= 1 << 47
            || !context.r8.is_multiple_of(16)
            || !stack_end.is_multiple_of(16)
            || !(context.r10..tls_end).contains(&context.r12)
            || !context.r13.is_multiple_of(4096)
            || (context.r13 != 0 && !context.r8.is_multiple_of(4096))
            || reserved_range(context.r8, stack_end)
            || reserved_range(context.r10, tls_end)
        {
            return Err(Error::InvalidArgument);
        }
        if context.r13 != 0 {
            let guard = context
                .r8
                .checked_sub(context.r13)
                .ok_or(Error::InvalidArgument)?;
            user_end(guard, context.r13)?;
            if reserved_range(guard, context.r8) {
                return Err(Error::InvalidArgument);
            }
        }
        Ok(Self {
            entry: context.rsi,
            argument: context.rdx,
            stack_base: context.r8,
            stack_bytes: context.r9,
            stack_end,
            tls_base: context.r10,
            tls_bytes: context.r11,
            fs_base: context.r12,
            guard_bytes: context.r13,
        })
    }
}

fn user_end(base: u64, bytes: u64) -> Result<u64, Error> {
    let end = base.checked_add(bytes).ok_or(Error::InvalidArgument)?;
    if base == 0 || bytes == 0 || end > 1 << 47 {
        return Err(Error::InvalidArgument);
    }
    Ok(end)
}

fn reserved_range(start: u64, end: u64) -> bool {
    end > super::THREAD_ARENA
        || super::USER_STACKS
            .into_iter()
            .any(|stack| start < stack + 16_384 && stack - 4096 < end)
        || super::USER_TLS
            .into_iter()
            .any(|tls| start < tls + 4096 && tls < end)
}

fn map_execution_error<E>(error: VmError<E>) -> Error {
    match error {
        VmError::InvalidAddress
        | VmError::InvalidContents
        | VmError::InvalidPermissions
        | VmError::AlreadyMapped
        | VmError::NotMapped => Error::InvalidArgument,
        VmError::InUse => Error::Busy,
        _ => Error::OutOfResources,
    }
}

fn create_thread<'kernel, 'objects>(
    caller: TaskId,
    start: Start,
    tasks: &mut Tasks<'kernel, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    region: &mut KernelRegion,
) -> Result<u64, Error> {
    let arena = ipc.arena.ok_or(Error::InvalidCall)?;
    let process = tasks.process_for(caller).ok_or(Error::PeerClosed)?;
    if tasks.reserve_thread(process).is_err() {
        return Err(Error::OutOfResources);
    }
    let caller_resources = tasks.thread_mut(caller).map_err(|_| Error::PeerClosed)?;
    let (pool, direct, memory) = (
        caller_resources.pool,
        caller_resources.direct,
        caller_resources.memory,
    );
    let owner = tasks.process_mut(process).map_err(|_| Error::PeerClosed)?;
    let space = owner.space.identity();
    if let Some(pending) = owner.pending_thread.as_mut() {
        if !pending.close_with(pool, region, &mut owner.space, |start, end| {
            ipc.invalidate_words(space, start, end);
        }) {
            return Err(Error::OutOfResources);
        }
        owner.pending_thread = None;
    }
    if let Some(pending) = owner.pending.as_mut() {
        if !pending.close(pool, region) {
            return Err(Error::OutOfResources);
        }
        owner.pending = None;
    }
    let entry = start.entry();
    if entry == 0
        || entry >= 1 << 47
        || owner.space.permissions(entry & !4095) != Some(Permissions::ReadExecute)
    {
        return Err(Error::InvalidArgument);
    }
    let charge = owner
        .account
        .as_ref()
        .map(|account| account.try_charge(Resource::Threads, 1))
        .transpose()
        .map_err(|_| Error::OutOfResources)?;
    let managed = matches!(start, Start::Managed { .. });
    let user_slot = if managed {
        (0..)
            .find(|slot| {
                !tasks
                    .process_threads(process)
                    .unwrap()
                    .any(|task| tasks.thread(task).unwrap().user_slot == *slot)
            })
            .ok_or(Error::OutOfResources)?
    } else {
        usize::MAX
    };
    let owner = tasks.process_mut(process).unwrap();
    if managed
        && user_pages(user_slot)
            .into_iter()
            .any(|page| owner.space.permissions(page).is_some())
    {
        return Err(Error::OutOfResources);
    }
    let first_slot = region
        .find_stack(pool, direct)
        .map_err(|_| Error::OutOfResources)?;
    let mut reservation = Some(owner.handles.reserve(1).map_err(super::native::map_error)?);
    let mut pending = PendingThread {
        stack: None,
        attempted: [false; 5],
        user_slot,
        thread_charge: charge,
    };
    let mut execution = None;
    let result = (|| {
        let extended = ExtendedState::new().map_err(|_| Error::InvalidArgument)?;
        let owner = tasks.process_mut(process).unwrap();
        let (argument, stack_end, tls_base, fs_base) = match &start {
            Start::Managed { argument, .. } => {
                let (stack, tls) = thread_addresses(user_slot).ok_or(Error::OutOfResources)?;
                for (index, page) in user_pages(user_slot).into_iter().enumerate() {
                    // Failed publication can still retain an alias owner.
                    pending.attempted[index] = true;
                    owner
                        .space
                        .map_page(page, Permissions::ReadWrite, &[])
                        .map_err(|_| Error::OutOfResources)?;
                }
                (*argument, stack + 16_384, tls, tls)
            }
            Start::Native(start) => {
                execution = Some(
                    owner
                        .space
                        .lease_execution(
                            start.stack_base,
                            start.stack_bytes,
                            start.tls_base,
                            start.tls_bytes,
                            start.guard_bytes,
                        )
                        .map_err(map_execution_error)?,
                );
                (
                    start.argument,
                    start.stack_end,
                    start.tls_base,
                    start.fs_base,
                )
            }
        };
        match KernelStack::try_new(pool, direct, region, first_slot) {
            Ok(stack) => pending.stack = Some(stack),
            Err(failure) => {
                pending.stack = failure.retained;
                return Err(Error::OutOfResources);
            }
        }
        let mut initial = UserContext::new(entry, stack_end).map_err(|_| Error::InvalidArgument)?;
        initial.rdi = argument;
        initial.rsi = tls_base;
        initial.rdx = u64::from(extended.avx_enabled());
        initial
            .set_fs_base(fs_base)
            .map_err(|_| Error::InvalidArgument)?;
        let control = arena
            .create(
                ObjectKind::ThreadControl,
                KernelObject::ThreadControl(RefCell::new(ProcessExit::growing())),
            )
            .map_err(|_| Error::OutOfResources)?;
        let thread_owner = control.try_clone().map_err(|_| Error::OutOfResources)?;
        let thread = ThreadResources {
            process_id: None,
            context: initial,
            stack: pending.stack.take().unwrap(),
            ticks: 0,
            exit_code: u64::MAX,
            exit_reason: ExitReason::Exited,
            wait: None,
            receive_slots: None,
            received_wait: None,
            received_handles: [None; 2],
            pool,
            memory,
            extended: Some(extended),
            execution: execution.take(),
            direct,
            user_slot,
            user_stack_owned: managed,
            thread_control: Some(thread_owner),
            thread_charge: pending.thread_charge.take(),
        };
        let task = match tasks.spawn_thread(process, thread) {
            Ok(task) => task,
            Err(mut failure) => {
                pending.thread_charge = failure.resources.thread_charge.take();
                pending.stack = Some(failure.resources.stack);
                execution = failure.resources.execution.take();
                return Err(Error::OutOfResources);
            }
        };
        let handles = tasks
            .process_mut(process)
            .unwrap()
            .handles
            .commit_transfer(
                reservation.take().unwrap(),
                TransferBatch::from_object(
                    control,
                    Rights::INSPECT
                        | Rights::CONTROL
                        | Rights::WAIT
                        | Rights::DUP
                        | Rights::TRANSFER,
                ),
            )
            .unwrap_or_else(|_| panic!("Reserved thread handle publication invariant"));
        stats.threads_created += 1;
        let _ = kryo_hal::print(format_args!(
            "THREAD: created id={:#x} process={:#x}\n",
            task.as_raw(),
            process.as_raw()
        ));
        Ok(handles[0].unwrap().as_raw())
    })();
    if result.is_err() {
        let owner = tasks.process_mut(process).unwrap();
        if let Some(lease) = execution.take() {
            owner
                .space
                .release_execution(&lease)
                .expect("Unpublished thread execution lease invariant");
        }
        let space = owner.space.identity();
        if !pending.close_with(pool, region, &mut owner.space, |start, end| {
            ipc.invalidate_words(space, start, end);
        }) {
            assert!(
                owner.pending_thread.is_none(),
                "One retained thread attempt invariant"
            );
            owner.pending_thread = Some(pending);
        }
    }
    if let Some(reservation) = reservation {
        tasks
            .process_mut(process)
            .unwrap()
            .handles
            .release_reservation(reservation)
            .unwrap_or_else(|_| panic!("Thread result reservation rollback invariant"));
    }
    result
}

pub(super) fn join<'objects>(
    caller: TaskId,
    control: Object<'objects>,
    deadline: Option<u64>,
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
) -> Result<Option<u64>, Error> {
    if deadline
        .is_some_and(|deadline| deadline <= kryo_hal::monotonic_time().expect("Missing join clock"))
    {
        return Err(Error::TimedOut);
    }
    let account = tasks.resources_mut(caller).unwrap().process.account.clone();
    let wait = ipc.begin_wait_for(caller, deadline, account.as_ref())?;
    tasks.thread_mut(caller).unwrap().wait = Some(wait);
    let result = control.with(|object| {
        let KernelObject::ThreadControl(exit) = object else {
            unreachable!()
        };
        exit.borrow_mut().wait(caller, wait, &mut ipc.waits)
    });
    ipc.observe(wait, control);
    if let Err(error) = result {
        ipc.waits
            .complete(
                wait,
                Completion::Failed {
                    error,
                    delivery: Delivery::NotDelivered,
                },
            )
            .expect("Rejected join completion invariant");
    }
    ipc.waits.park(wait).expect("Thread join handoff invariant");
    Ok(None)
}

impl Drop for PendingThread {
    fn drop(&mut self) {
        if self.stack.is_some() || self.attempted.iter().any(|attempted| *attempted) {
            core::mem::forget(self.thread_charge.take());
        }
    }
}
