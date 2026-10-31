use super::ipc_runtime::{IpcRuntime, map_error as map_ipc_error};
use super::{CallStats, KernelObject, Object, Tasks};
use kryo_core::capability::{CapabilityError, Handle, Rights};
use kryo_core::ipc::{Completion, Delivery, Message, ReplyToken};
use kryo_core::object::ObjectKind;
use kryo_core::sched::TaskState;
use kryo_core::task::TaskId;
use kryo_hal::UserContext;
use kryo_interfaces::syscall::{Call, Error, TaskStatus};

pub(super) fn dispatch<'objects>(
    caller: TaskId,
    tasks: &mut Tasks<'_, 'objects>,
    stats: &mut CallStats,
    ipc: &mut IpcRuntime<'objects>,
) {
    let context = tasks.resources_mut(caller).expect("Missing caller").context;
    let result = Call::try_from(context.rax).and_then(|call| {
        let handle = Handle::from_raw(context.rdi);
        match call {
            Call::Yield => {
                stats.yields += 1;
                Ok(Some(0))
            }
            Call::Exit => {
                tasks.resources_mut(caller).unwrap().exit_code = context.rdi;
                tasks.stop(caller).map_err(|_| Error::PeerClosed)?;
                Ok(Some(0))
            }
            Call::Close => tasks
                .resources_mut(caller)
                .unwrap()
                .handles
                .close(handle)
                .map(|()| Some(0))
                .map_err(map_error),
            Call::Duplicate => {
                let rights = Rights::from_bits(context.rsi).ok_or(Error::InvalidArgument)?;
                tasks
                    .resources_mut(caller)
                    .unwrap()
                    .handles
                    .duplicate(handle, rights)
                    .map(|handle| Some(handle.as_raw()))
                    .map_err(map_error)
            }
            Call::TaskStatus | Call::TaskStop | Call::CancelWait => {
                let rights = if call == Call::TaskStatus {
                    Rights::INSPECT
                } else {
                    Rights::CONTROL
                };
                let object = lookup(tasks, caller, handle, ObjectKind::TaskControl, rights)?;
                let target = object
                    .with(|object| match object {
                        KernelObject::TaskControl(task) => Some(*task),
                        _ => None,
                    })
                    .ok_or(Error::WrongType)?;
                if call == Call::TaskStatus {
                    let state = match tasks.state(target) {
                        Ok(TaskState::Runnable) => TaskStatus::Runnable,
                        Ok(TaskState::Running) => TaskStatus::Running,
                        Ok(TaskState::Blocked) => TaskStatus::Blocked,
                        Ok(TaskState::Dying) => TaskStatus::Dying,
                        Err(_) => TaskStatus::Exited,
                    };
                    Ok(Some(state as u64))
                } else if call == Call::CancelWait {
                    let wait = tasks
                        .resources_mut(target)
                        .map_err(|_| Error::PeerClosed)?
                        .wait
                        .ok_or(Error::InvalidArgument)?;
                    ipc.cancel(wait)
                        .map(|delivery| Some(u64::from(delivery == Delivery::Accepted)))
                        .map_err(map_ipc_error)
                } else {
                    tasks.stop(target).map_err(|_| Error::PeerClosed)?;
                    tasks.resources_mut(target).unwrap().exit_code = context.rsi;
                    Ok(Some(0))
                }
            }
            Call::Send | Call::Receive | Call::Call | Call::Wait => {
                let (kind, rights) = match call {
                    Call::Receive => (ObjectKind::Endpoint, Rights::RECEIVE),
                    Call::Wait => (ObjectKind::Notification, Rights::WAIT),
                    _ => (ObjectKind::Endpoint, Rights::SEND),
                };
                let object = lookup(tasks, caller, handle, kind, rights)?;
                wait_on(caller, call, context, &object, tasks, ipc)
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
    context: UserContext,
    object: &Object<'objects>,
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
) -> Result<Option<u64>, Error> {
    let wait = ipc.waits.begin(caller).map_err(|_| Error::OutOfResources)?;
    tasks.resources_mut(caller).unwrap().wait = Some(wait);
    let result = object.with(|object| match (call, object) {
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
        (Call::Receive, KernelObject::Endpoint(endpoint)) => {
            endpoint.borrow_mut().receive(caller, wait, &mut ipc.waits)
        }
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
        .handles
        .lookup(handle, kind, rights)
        .map_err(map_error)
}

fn map_error(error: CapabilityError) -> Error {
    match error {
        CapabilityError::InvalidHandle => Error::InvalidHandle,
        CapabilityError::WrongKind => Error::WrongType,
        CapabilityError::PermissionDenied => Error::PermissionDenied,
        CapabilityError::Capacity | CapabilityError::ReferenceLimit => Error::OutOfResources,
    }
}
