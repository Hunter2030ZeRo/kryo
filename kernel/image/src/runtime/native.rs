use super::ipc_runtime::{IpcRuntime, map_error as map_ipc_error};
use super::{CallStats, KernelObject, Memory, Object, SharedLease, Tasks};
use crate::runtime::KernelRegion;
use kryo_core::capability::{CapabilityError, Handle, Rights, TransferSpec};
use kryo_core::ipc::{Completion, Delivery, Message, ReplyToken};
use kryo_core::mm::shared::SharedError;
use kryo_core::mm::vm::{Permissions, VmError};
use kryo_core::object::ObjectKind;
use kryo_core::process::{ExitReason, ExitStatus, ProcessId};
use kryo_core::sched::TaskState;
use kryo_core::task::TaskId;
use kryo_hal::UserContext;
use kryo_interfaces::syscall::{Call, Error, TaskStatus};

pub(crate) fn dispatch<'objects>(
    caller: TaskId,
    tasks: &mut Tasks<'_, 'objects>,
    stats: &mut CallStats,
    ipc: &mut IpcRuntime<'objects>,
    region: Option<&mut KernelRegion>,
) {
    let context = tasks.resources_mut(caller).expect("Missing caller").context;
    let result = Call::try_from(context.rax).and_then(|call| {
        let handle = Handle::from_raw(context.rdi);
        match call {
            Call::ManifestCreate => {
                let arena = ipc.arena.ok_or(Error::InvalidCall)?;
                let owner = tasks.process_for(caller).ok_or(Error::PeerClosed)?;
                let handles = &mut tasks.resources_mut(caller).unwrap().process.handles;
                super::startup::create(
                    owner,
                    context.rdi,
                    kryo_interfaces::startup::Metadata {
                        service: context.rsi,
                        instance: context.rdx,
                        argument: context.r8,
                    },
                    handles,
                    arena,
                )
                .map(|handle| Some(handle.as_raw()))
            }
            Call::ManifestGrant => {
                let owner = tasks.process_for(caller).ok_or(Error::PeerClosed)?;
                let entry = kryo_interfaces::startup::Entry {
                    role: context.rsi,
                    kind: kryo_interfaces::startup::ObjectKind::try_from(context.r8)
                        .map_err(|_| Error::InvalidArgument)?,
                    rights: context.r9,
                    handle: context.rdx,
                };
                let resources = tasks.resources_mut(caller).unwrap();
                super::startup::grant(owner, handle, entry, &resources.process.handles)
                    .map(|()| Some(0))
            }
            Call::StartupRead => {
                let object = lookup(
                    tasks,
                    caller,
                    handle,
                    ObjectKind::StartupRecord,
                    Rights::READ,
                )?;
                super::startup::read(&object, context.rsi).map(Some)
            }
            Call::HandleInfo => {
                let mut resources = tasks.resources_mut(caller).unwrap();
                let (kind, rights) = resources
                    .process
                    .handles
                    .describe(handle)
                    .map_err(map_error)?;
                resources.context.r8 = rights.bits();
                Ok(Some(super::startup::wire_kind(kind) as u64))
            }
            Call::ThreadCreate => super::threads::create(
                caller,
                context,
                tasks,
                ipc,
                stats,
                region.ok_or(Error::InvalidCall)?,
            )
            .map(Some),
            Call::ThreadExit => {
                let thread = tasks.thread_mut(caller).unwrap();
                thread.exit_code = context.rdi;
                thread.exit_reason = kryo_core::process::ExitReason::Exited;
                tasks.stop(caller).map_err(|_| Error::PeerClosed)?;
                Ok(Some(0))
            }
            Call::ThreadJoin | Call::ThreadJoinUntil => {
                let control = lookup(
                    tasks,
                    caller,
                    handle,
                    ObjectKind::ThreadControl,
                    Rights::WAIT,
                )?;
                super::threads::join(
                    caller,
                    control,
                    (call == Call::ThreadJoinUntil).then_some(context.rdx),
                    tasks,
                    ipc,
                )
            }
            Call::Spawn => super::process::spawn(
                caller,
                context,
                tasks,
                ipc,
                stats,
                region.ok_or(Error::InvalidCall)?,
            )
            .map(Some),
            Call::SpawnManifest => super::process::spawn_manifest(
                caller,
                context,
                tasks,
                ipc,
                stats,
                region.ok_or(Error::InvalidCall)?,
            )
            .map(Some),
            Call::ProcessWait | Call::ProcessWaitUntil => {
                let object = lookup(
                    tasks,
                    caller,
                    handle,
                    ObjectKind::ProcessControl,
                    Rights::WAIT,
                )?;
                super::process::wait(
                    caller,
                    object,
                    (call == Call::ProcessWaitUntil).then_some(context.rdx),
                    tasks,
                    ipc,
                )
            }
            Call::DebugLog => {
                lookup(
                    tasks,
                    caller,
                    handle,
                    ObjectKind::DebugConsole,
                    Rights::WRITE,
                )?;
                let _ = kryo_hal::print(format_args!(
                    "USERLOG: process={:#x} event={} value={}\n",
                    caller.as_raw(),
                    context.rsi,
                    context.r8
                ));
                stats.last_debug = Some((caller, context.rsi, context.r8));
                Ok(Some(0))
            }
            Call::Yield => {
                stats.yields += 1;
                Ok(Some(0))
            }
            Call::ClockRead => Ok(Some(now())),
            Call::SleepUntil => {
                if context.rdi <= now() {
                    return Ok(Some(0));
                }
                let wait = ipc.sleep(caller, context.rdi)?;
                tasks.resources_mut(caller).unwrap().wait = Some(wait);
                ipc.waits.park(wait).expect("Cannot finish sleep handoff");
                Ok(None)
            }
            Call::SetTls => {
                tasks
                    .resources_mut(caller)
                    .unwrap()
                    .context
                    .set_fs_base(context.rdi)
                    .map_err(|_| Error::InvalidArgument)?;
                stats.tls_sets += 1;
                Ok(Some(0))
            }
            Call::Exit => {
                let process = tasks.process_for(caller).ok_or(Error::PeerClosed)?;
                tasks
                    .stop_process(
                        process,
                        ExitStatus {
                            reason: ExitReason::Exited,
                            code: context.rdi,
                        },
                    )
                    .map_err(|_| Error::PeerClosed)?;
                Ok(Some(0))
            }
            Call::Close => tasks
                .resources_mut(caller)
                .unwrap()
                .process
                .handles
                .close(handle)
                .map(|()| Some(0))
                .map_err(map_error),
            Call::Duplicate => {
                let rights = Rights::from_bits(context.rsi).ok_or(Error::InvalidArgument)?;
                tasks
                    .resources_mut(caller)
                    .unwrap()
                    .process
                    .handles
                    .duplicate(handle, rights)
                    .map(|handle| Some(handle.as_raw()))
                    .map_err(map_error)
            }
            Call::EndpointCreate | Call::NotificationCreate => {
                let handles = &mut tasks.resources_mut(caller).unwrap().process.handles;
                super::channels::create(caller, handle, call, handles, ipc)
                    .map(|handle| Some(handle.as_raw()))
            }
            Call::MemoryCreate => {
                let arena = ipc.arena.ok_or(Error::InvalidCall)?;
                let pages = usize::try_from(context.rdi).map_err(|_| Error::InvalidArgument)?;
                let resources = tasks.resources_mut(caller).unwrap();
                let memory =
                    Memory::new(resources.pool, pages, &mut *resources.memory.borrow_mut())
                        .map_err(|error| match error {
                            SharedError::InvalidSize => Error::InvalidArgument,
                            SharedError::Allocation(_) | SharedError::Backend(_) => {
                                Error::OutOfResources
                            }
                        })?;
                let object = arena
                    .create(ObjectKind::Memory, KernelObject::Memory(memory))
                    .map_err(|_| Error::OutOfResources)?;
                resources
                    .process
                    .handles
                    .insert(
                        object,
                        Rights::READ | Rights::WRITE | Rights::MAP | Rights::DUP | Rights::TRANSFER,
                    )
                    .map(|handle| Some(handle.as_raw()))
                    .map_err(|error| map_error(error.error))
            }
            Call::DeviceRead | Call::DeviceWrite => {
                let rights = if call == Call::DeviceWrite {
                    Rights::WRITE
                } else {
                    Rights::READ
                };
                let object = lookup(tasks, caller, handle, ObjectKind::Device, rights)?;
                let owner = tasks.process_for(caller).ok_or(Error::PeerClosed)?;
                super::devices::access(
                    &object,
                    owner,
                    context.rsi,
                    (call == Call::DeviceWrite).then_some(context.rdx),
                )
                .map(Some)
            }
            Call::MemoryMap => {
                if super::threads::reserved_address(context.rsi) {
                    return Err(Error::InvalidArgument);
                }
                let permissions = match context.r8 {
                    0 => Permissions::ReadOnly,
                    1 => Permissions::ReadWrite,
                    _ => return Err(Error::InvalidArgument),
                };
                let mut rights = Rights::READ | Rights::MAP;
                if permissions.writable() {
                    rights = rights | Rights::WRITE;
                }
                let object = lookup(tasks, caller, handle, ObjectKind::Memory, rights)?;
                let page = usize::try_from(context.rdx).map_err(|_| Error::InvalidArgument)?;
                let physical = object
                    .with(|object| match object {
                        KernelObject::Memory(memory) => memory.page_address(page),
                        _ => None,
                    })
                    .ok_or(Error::InvalidArgument)?;
                let lease = SharedLease {
                    _owner: object,
                    physical,
                };
                tasks
                    .resources_mut(caller)
                    .unwrap()
                    .process
                    .space
                    .map_shared_page(context.rsi, permissions, lease)
                    .map(|()| Some(0))
                    .map_err(map_vm_error)
            }
            Call::MemoryUnmap => {
                if super::threads::reserved_address(context.rdi) {
                    return Err(Error::InvalidArgument);
                }
                tasks
                    .resources_mut(caller)
                    .unwrap()
                    .process
                    .space
                    .unmap_page(context.rdi)
                    .map(|()| Some(0))
                    .map_err(map_vm_error)
            }
            Call::TaskStatus | Call::TaskStop | Call::CancelWait => {
                let rights = if call == Call::TaskStatus {
                    Rights::INSPECT
                } else {
                    Rights::CONTROL
                };
                let object = {
                    let resources = tasks.resources_mut(caller).unwrap();
                    let handles = &resources.process.handles;
                    let mut object = handles.lookup(handle, ObjectKind::TaskControl, rights);
                    for kind in [ObjectKind::ProcessControl, ObjectKind::ThreadControl] {
                        if matches!(object, Err(CapabilityError::WrongKind)) {
                            object = handles.lookup(handle, kind, rights);
                        }
                    }
                    object.map_err(map_error)?
                };
                let (target, terminated) = object
                    .with(|object| match object {
                        KernelObject::TaskControl(task) => {
                            Some((ControlTarget::Thread(*task), false))
                        }
                        KernelObject::ThreadControl(exit) => {
                            let exit = exit.borrow();
                            exit.task()
                                .map(|task| (ControlTarget::Thread(task), exit.result().is_some()))
                        }
                        KernelObject::ProcessControl(control) => {
                            let control = control.borrow();
                            control.process.map(|process| {
                                (ControlTarget::Process(process), control.result().is_some())
                            })
                        }
                        _ => None,
                    })
                    .ok_or(Error::WrongType)?;
                if call == Call::TaskStatus {
                    let state = if terminated {
                        TaskStatus::Exited
                    } else {
                        match target {
                            ControlTarget::Thread(task) => task_status(tasks.state(task)),
                            ControlTarget::Process(process) => process_status(tasks, process),
                        }
                    };
                    return Ok(Some(state as u64));
                }
                if terminated {
                    return Err(Error::PeerClosed);
                }
                if call == Call::CancelWait {
                    let task = match target {
                        ControlTarget::Thread(task) => task,
                        ControlTarget::Process(process) => {
                            let mut waiting = None;
                            for task in tasks
                                .process_threads(process)
                                .map_err(|_| Error::PeerClosed)?
                            {
                                if tasks.thread(task).unwrap().wait.is_some() {
                                    if waiting.is_some() {
                                        return Err(Error::InvalidArgument);
                                    }
                                    waiting = Some(task);
                                }
                            }
                            waiting.ok_or(Error::InvalidArgument)?
                        }
                    };
                    let wait = tasks
                        .thread_mut(task)
                        .map_err(|_| Error::PeerClosed)?
                        .wait
                        .ok_or(Error::InvalidArgument)?;
                    ipc.cancel(wait)
                        .map(|delivery| Some(u64::from(delivery == Delivery::Accepted)))
                        .map_err(map_ipc_error)
                } else {
                    let status = ExitStatus {
                        reason: ExitReason::Stopped,
                        code: context.rsi,
                    };
                    match target {
                        ControlTarget::Process(process) => tasks
                            .stop_process(process, status)
                            .map_err(|_| Error::PeerClosed)?,
                        ControlTarget::Thread(task) => {
                            let thread = tasks.thread_mut(task).map_err(|_| Error::PeerClosed)?;
                            thread.exit_code = status.code;
                            thread.exit_reason = status.reason;
                            tasks.stop(task).map_err(|_| Error::PeerClosed)?;
                        }
                    }
                    Ok(Some(0))
                }
            }
            Call::Send
            | Call::Receive
            | Call::Call
            | Call::Wait
            | Call::SendCaps
            | Call::CallCaps
            | Call::SendUntil
            | Call::ReceiveUntil
            | Call::ReceiveFor
            | Call::CallUntil
            | Call::WaitUntil
            | Call::SendCapsUntil
            | Call::CallCapsUntil => {
                let (operation, mut deadline) = match call {
                    Call::SendUntil => (Call::Send, Some(context.rdx)),
                    Call::ReceiveUntil => (Call::Receive, Some(context.rdx)),
                    Call::ReceiveFor => (Call::Receive, None),
                    Call::CallUntil => (Call::Call, Some(context.rdx)),
                    Call::WaitUntil => (Call::Wait, Some(context.rdx)),
                    Call::SendCapsUntil => (Call::SendCaps, Some(context.rdx)),
                    Call::CallCapsUntil => (Call::CallCaps, Some(context.rdx)),
                    _ => (call, None),
                };
                let (kind, rights) = match operation {
                    Call::Receive => (ObjectKind::Endpoint, Rights::RECEIVE),
                    Call::Wait => (ObjectKind::Notification, Rights::WAIT),
                    _ => (ObjectKind::Endpoint, Rights::SEND),
                };
                let object = lookup(tasks, caller, handle, kind, rights)?;
                if call == Call::ReceiveFor || deadline.is_some() {
                    let admitted = now();
                    if call == Call::ReceiveFor {
                        deadline = Some(
                            admitted
                                .checked_add(context.rdx)
                                .ok_or(Error::InvalidArgument)?,
                        );
                    }
                    if deadline.is_some_and(|deadline| deadline <= admitted) {
                        #[cfg(feature = "boot-tests")]
                        if call == Call::ReceiveUntil {
                            stats.expired_receive_admissions += 1;
                        }
                        return Err(Error::TimedOut);
                    }
                }
                wait_on(caller, operation, deadline, context, &object, tasks, ipc)
            }
            Call::Reply | Call::EndpointClose | Call::Signal => {
                let (kind, rights) = match call {
                    Call::Reply => (ObjectKind::Endpoint, Rights::RECEIVE),
                    Call::EndpointClose => (ObjectKind::Endpoint, Rights::CONTROL),
                    _ => (ObjectKind::Notification, Rights::SIGNAL),
                };
                let object = lookup(tasks, caller, handle, kind, rights)?;
                object
                    .with(|object| match (call, object) {
                        (Call::Reply, KernelObject::Endpoint(endpoint)) => endpoint
                            .borrow_mut()
                            .reply(
                                caller,
                                ReplyToken::from_raw(context.rsi),
                                message(context),
                                &mut ipc.waits,
                            )
                            .map_err(map_ipc_error),
                        (Call::EndpointClose, KernelObject::Endpoint(endpoint)) => {
                            endpoint.borrow_mut().close(&mut ipc.waits);
                            Ok(())
                        }
                        (Call::Signal, KernelObject::Notification(notification)) => notification
                            .borrow_mut()
                            .signal(&mut ipc.waits)
                            .map_err(map_ipc_error),
                        _ => Err(Error::WrongType),
                    })
                    .map(|()| Some(0))
            }
        }
    });

    let context = &mut tasks.resources_mut(caller).unwrap().context;
    match result {
        Ok(None) => return,
        Ok(Some(value)) => {
            context.rax = 0;
            context.rdx = value;
        }
        Err(error) => {
            #[cfg(feature = "boot-tests")]
            {
                if let Ok(call) = Call::try_from(context.rax) {
                    stats.last_failure = Some((caller, call, error));
                }
            }
            if error == Error::PermissionDenied {
                stats.denied += 1;
            }
            if error == Error::InvalidHandle {
                stats.invalid += 1;
            }
            context.rax = error.status();
            context.rdx = 0;
        }
    }
    if tasks.state(caller) == Ok(TaskState::Blocked) {
        tasks.wake(caller).expect("Cannot wake completed call");
    }
}

fn wait_on<'objects>(
    caller: TaskId,
    call: Call,
    deadline: Option<u64>,
    context: UserContext,
    object: &Object<'objects>,
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
) -> Result<Option<u64>, Error> {
    let mut resources = tasks.resources_mut(caller).unwrap();
    let reservation = if call == Call::Receive {
        let count = usize::try_from(context.rsi).map_err(|_| Error::InvalidArgument)?;
        Some(
            resources
                .process
                .handles
                .reserve(count)
                .map_err(map_error)?,
        )
    } else {
        None
    };
    let batch = if matches!(call, Call::SendCaps | Call::CallCaps) {
        let count = usize::try_from(context.rsi).map_err(|_| Error::InvalidArgument)?;
        if !(1..=2).contains(&count) {
            return Err(Error::InvalidArgument);
        }
        let raw_handles = [context.r12, context.r13];
        let raw_rights = [context.r14, context.r15];
        let mut specs = [TransferSpec {
            handle: Handle::INVALID,
            rights: Rights::READ,
        }; 2];
        for index in 0..count {
            specs[index] = TransferSpec {
                handle: Handle::from_raw(raw_handles[index]),
                rights: Rights::from_bits(raw_rights[index]).ok_or(Error::InvalidArgument)?,
            };
        }
        Some(
            resources
                .process
                .handles
                .prepare_transfer(&specs[..count])
                .map_err(map_error)?,
        )
    } else {
        None
    };
    let registration = ipc.begin_wait(caller, deadline);
    let wait = match registration {
        Ok(wait) => wait,
        Err(_) => {
            if let Some(reservation) = reservation {
                resources
                    .process
                    .handles
                    .release_reservation(reservation)
                    .unwrap_or_else(|_| panic!("Receive rollback invariant"));
            }
            return Err(Error::OutOfResources);
        }
    };
    resources.wait = Some(wait);
    resources.receive_slots = reservation;
    if let Some(batch) = batch {
        ipc.hold_transfer(caller, wait, batch);
    }
    let result =
        object.with(|object| match (call, object) {
            (Call::Send, KernelObject::Endpoint(endpoint)) => {
                endpoint
                    .borrow_mut()
                    .send(caller, wait, message(context), &mut ipc.waits)
            }
            (Call::Call, KernelObject::Endpoint(endpoint)) => {
                endpoint
                    .borrow_mut()
                    .call(caller, wait, message(context), &mut ipc.waits)
            }
            (Call::SendCaps, KernelObject::Endpoint(endpoint)) => endpoint.borrow_mut().send_caps(
                caller,
                wait,
                message(context),
                context.rsi as usize,
                &mut ipc.waits,
            ),
            (Call::CallCaps, KernelObject::Endpoint(endpoint)) => endpoint.borrow_mut().call_caps(
                caller,
                wait,
                message(context),
                context.rsi as usize,
                &mut ipc.waits,
            ),
            (Call::Receive, KernelObject::Endpoint(endpoint)) => endpoint
                .borrow_mut()
                .receive_caps(caller, wait, context.rsi as usize, &mut ipc.waits),
            (Call::Wait, KernelObject::Notification(notification)) => {
                notification.borrow_mut().wait(caller, wait, &mut ipc.waits)
            }
            _ => unreachable!("Capability kind and call disagree"),
        });
    if let Err(error) = result {
        ipc.waits
            .complete(
                wait,
                Completion::Failed {
                    error,
                    delivery: Delivery::NotDelivered,
                },
            )
            .expect("Failed IPC operation completed its wait");
    }
    ipc.waits.park(wait).expect("Cannot finish IPC handoff");
    Ok(None)
}

fn now() -> u64 {
    kryo_hal::monotonic_time().expect("Missing native monotonic clock")
}

fn map_vm_error<E>(error: VmError<E>) -> Error {
    match error {
        VmError::Allocation(_) | VmError::MappingCapacity | VmError::TableCapacity => {
            Error::OutOfResources
        }
        _ => Error::InvalidArgument,
    }
}

fn message(context: UserContext) -> Message {
    Message([context.r8, context.r9, context.r10, context.r11])
}

fn lookup<'objects>(
    tasks: &mut Tasks<'_, 'objects>,
    caller: TaskId,
    handle: Handle,
    kind: ObjectKind,
    rights: Rights,
) -> Result<Object<'objects>, Error> {
    tasks
        .resources_mut(caller)
        .unwrap()
        .process
        .handles
        .lookup(handle, kind, rights)
        .map_err(map_error)
}

pub(super) fn map_error(error: CapabilityError) -> Error {
    match error {
        CapabilityError::InvalidHandle => Error::InvalidHandle,
        CapabilityError::WrongKind => Error::WrongType,
        CapabilityError::PermissionDenied => Error::PermissionDenied,
        CapabilityError::Capacity | CapabilityError::ReferenceLimit => Error::OutOfResources,
    }
}

#[derive(Clone, Copy)]
enum ControlTarget {
    Thread(TaskId),
    Process(ProcessId),
}

fn task_status(state: Result<TaskState, kryo_core::task::TaskError>) -> TaskStatus {
    match state {
        Ok(TaskState::Runnable) => TaskStatus::Runnable,
        Ok(TaskState::Running) => TaskStatus::Running,
        Ok(TaskState::Blocked) => TaskStatus::Blocked,
        Ok(TaskState::Dying) => TaskStatus::Dying,
        Err(_) => TaskStatus::Exited,
    }
}

fn process_status(tasks: &Tasks<'_, '_>, process: ProcessId) -> TaskStatus {
    let Ok(members) = tasks.process_threads(process) else {
        return TaskStatus::Exited;
    };
    let mut state = TaskStatus::Dying;
    for task in members {
        match tasks.state(task) {
            Ok(TaskState::Running) => return TaskStatus::Running,
            Ok(TaskState::Runnable) => state = TaskStatus::Runnable,
            Ok(TaskState::Blocked) if state != TaskStatus::Runnable => state = TaskStatus::Blocked,
            _ => {}
        }
    }
    state
}
