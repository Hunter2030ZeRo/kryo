use super::{KernelObject, Object, Tasks};
use kryo_core::ipc::{Completion, Delivery, IpcError};
use kryo_core::sched::TaskState;
use kryo_core::sync::wait::{WaitId, WaitTable};
use kryo_core::task::TaskId;
use kryo_interfaces::syscall::Error;

pub(super) struct IpcRuntime<'objects> {
    pub(super) waits: WaitTable<Completion, 2>,
    objects: [Option<Object<'objects>>; 4],
    pub(super) accepted_failures: usize,
}

impl<'objects> IpcRuntime<'objects> {
    pub(super) fn new() -> Self {
        Self {
            waits: WaitTable::new(),
            objects: [const { None }; 4],
            accepted_failures: 0,
        }
    }

    pub(super) fn register(&mut self, object: Object<'objects>) {
        let slot = self.objects.iter_mut().find(|slot| slot.is_none());
        *slot.expect("IPC fixture registry full") = Some(object);
    }

    pub(super) fn cancel(&mut self, wait: WaitId) -> Result<Delivery, IpcError> {
        for object in self.objects.iter().flatten() {
            let result = object.with(|object| match object {
                KernelObject::Endpoint(endpoint) => {
                    endpoint.borrow_mut().cancel(wait, &mut self.waits)
                }
                KernelObject::Notification(notification) => notification
                    .borrow_mut()
                    .cancel(wait, &mut self.waits)
                    .map(|()| Delivery::NotDelivered),
                KernelObject::TaskControl(_) => Err(IpcError::InvalidWait),
            });
            if result.is_ok() {
                return result;
            }
        }
        Err(IpcError::InvalidWait)
    }

    pub(super) fn settle(&mut self, tasks: &mut Tasks<'_, 'objects>) {
        let mut dying = [None; 2];
        for (slot, id) in dying.iter_mut().zip(
            tasks
                .ids()
                .filter(|id| tasks.state(*id) == Ok(TaskState::Dying)),
        ) {
            *slot = Some(id);
        }
        for id in dying.into_iter().flatten() {
            self.task_died(id);
            if let Some(wait) = tasks.resources_mut(id).unwrap().wait.take() {
                self.waits.abandon(wait).expect("Missing dying task wait");
            }
        }

        // Object callbacks have released their locks. Save return values before
        // publishing Runnable; the next dispatch may immediately consume them.
        while let Some((wait, id)) = self.waits.next_ready() {
            assert_eq!(tasks.state(id), Ok(TaskState::Blocked));
            let resources = tasks.resources_mut(id).unwrap();
            assert_eq!(resources.wait.take(), Some(wait));
            let completion = self.waits.take(wait).expect("Missing IPC completion");
            let context = &mut resources.context;
            context.rax = 0;
            context.rdx = 0;
            match completion {
                Completion::Sent | Completion::Notified => {}
                Completion::Received { message, reply } => {
                    context.rdx = reply.map_or(0, |reply| reply.as_raw());
                    [context.r8, context.r9, context.r10, context.r11] = message.0;
                }
                Completion::Replied(message) => {
                    [context.r8, context.r9, context.r10, context.r11] = message.0;
                }
                Completion::Failed { error, delivery } => {
                    self.accepted_failures += usize::from(delivery == Delivery::Accepted);
                    context.rax = map_error(error).status();
                    context.rdx = u64::from(delivery == Delivery::Accepted);
                }
            }
            tasks.wake(id).expect("Cannot wake IPC completion");
        }
    }

    fn task_died(&mut self, id: TaskId) {
        for object in self.objects.iter().flatten() {
            object.with(|object| match object {
                KernelObject::Endpoint(endpoint) => {
                    endpoint.borrow_mut().task_died(id, &mut self.waits);
                }
                KernelObject::Notification(notification) => {
                    notification.borrow_mut().task_died(id, &mut self.waits);
                }
                KernelObject::TaskControl(_) => {}
            });
        }
    }
}

pub(super) fn map_error(error: IpcError) -> Error {
    match error {
        IpcError::Closed | IpcError::PeerClosed => Error::PeerClosed,
        IpcError::Capacity => Error::OutOfResources,
        IpcError::PermissionDenied => Error::PermissionDenied,
        IpcError::InvalidReply => Error::InvalidReply,
        IpcError::Cancelled => Error::Cancelled,
        IpcError::InvalidWait | IpcError::AlreadyWaiting => Error::InvalidArgument,
    }
}
