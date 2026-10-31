use super::{Completion, Delivery, IpcError};
use crate::sync::wait::{WaitId, WaitTable};
use crate::task::TaskId;

/// One consumer and a coalescing pending bit, serialized with its WaitTable.
pub struct Notification {
    owner: TaskId,
    pending: bool,
    waiting: Option<WaitId>,
    closed: Option<IpcError>,
}

impl Notification {
    pub const fn new(owner: TaskId) -> Self {
        Self {
            owner,
            pending: false,
            waiting: None,
            closed: None,
        }
    }

    pub fn wait<const WAITS: usize>(
        &mut self,
        consumer: TaskId,
        wait: WaitId,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        if consumer != self.owner {
            return Err(IpcError::PermissionDenied);
        }
        if let Some(error) = self.closed {
            return Err(error);
        }
        if waits.task(wait) != Ok(consumer) || !waits.is_pending(wait) {
            return Err(IpcError::InvalidWait);
        }
        self.prune(waits);
        if self.waiting.is_some() {
            return Err(IpcError::AlreadyWaiting);
        }
        if self.pending {
            self.pending = false;
            let _ = waits.complete(wait, Completion::Notified);
        } else {
            self.waiting = Some(wait);
        }
        Ok(())
    }

    pub fn signal<const WAITS: usize>(
        &mut self,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        if let Some(error) = self.closed {
            return Err(error);
        }
        self.prune(waits);
        if let Some(wait) = self.waiting.take() {
            let _ = waits.complete(wait, Completion::Notified);
        } else {
            self.pending = true;
        }
        Ok(())
    }

    pub fn cancel<const WAITS: usize>(
        &mut self,
        wait: WaitId,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        self.prune(waits);
        if self.waiting != Some(wait) {
            return Err(IpcError::InvalidWait);
        }
        self.waiting = None;
        let _ = waits.complete(
            wait,
            Completion::Failed {
                error: IpcError::Cancelled,
                delivery: Delivery::NotDelivered,
            },
        );
        Ok(())
    }

    pub fn close<const WAITS: usize>(&mut self, waits: &mut WaitTable<Completion, WAITS>) {
        self.shutdown(IpcError::Closed, waits);
    }

    pub fn task_died<const WAITS: usize>(
        &mut self,
        task: TaskId,
        waits: &mut WaitTable<Completion, WAITS>,
    ) {
        if task == self.owner {
            self.shutdown(IpcError::PeerClosed, waits);
        }
    }

    pub const fn is_closed(&self) -> bool {
        self.closed.is_some()
    }

    fn prune<const WAITS: usize>(&mut self, waits: &WaitTable<Completion, WAITS>) {
        if self.waiting.is_some_and(|wait| !waits.is_pending(wait)) {
            self.waiting = None;
        }
    }

    fn shutdown<const WAITS: usize>(
        &mut self,
        error: IpcError,
        waits: &mut WaitTable<Completion, WAITS>,
    ) {
        if self.closed.is_some() {
            return;
        }
        self.closed = Some(error);
        self.pending = false;
        if let Some(wait) = self.waiting.take() {
            let _ = waits.complete(
                wait,
                Completion::Failed {
                    error,
                    delivery: Delivery::NotDelivered,
                },
            );
        }
    }
}
