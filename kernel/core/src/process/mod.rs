use crate::ipc::{Completion, Delivery, IpcError};
use crate::storage::Slots;
use crate::sync::wait::{WaitId, WaitTable};
use crate::task::TaskId;

mod table;
pub use table::{
    ProcessId, ProcessInsertError, ProcessReapError, ProcessState, ProcessTable, ProcessTableError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum ExitReason {
    Exited = 0,
    Fault = 1,
    Stopped = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitStatus {
    pub reason: ExitReason,
    pub code: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessError {
    AlreadyBound,
    Unbound,
    AlreadyExited,
}

#[derive(Clone, Copy)]
struct Observer {
    task: TaskId,
    wait: WaitId,
}

/// Durable exit observation, independent of execution resources and task slots.
///
/// The runtime retains this object while any control handle or observer exists.
/// One persistent WaitTable owns its registrations. Serialize all access and
/// detach observers before abandoning their waits or replacing that table.
/// A bound TaskId describes one execution generation; it grants no authority.
pub struct ProcessExit<const OBSERVERS: usize> {
    task: Option<TaskId>,
    result: Option<ExitStatus>,
    observers: Slots<Option<Observer>, OBSERVERS>,
}

impl<const OBSERVERS: usize> Default for ProcessExit<OBSERVERS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const OBSERVERS: usize> ProcessExit<OBSERVERS> {
    pub const fn new() -> Self {
        Self {
            task: None,
            result: None,
            observers: Slots::new([None; OBSERVERS]),
        }
    }

    pub const fn growing() -> Self {
        Self {
            task: None,
            result: None,
            observers: Slots::growing(),
        }
    }

    /// Bind exactly once, before publishing the process control capability.
    pub fn bind(&mut self, task: TaskId) -> Result<(), ProcessError> {
        if self.task.is_some() {
            return Err(ProcessError::AlreadyBound);
        }
        self.task = Some(task);
        Ok(())
    }

    pub const fn task(&self) -> Option<TaskId> {
        self.task
    }

    pub const fn result(&self) -> Option<ExitStatus> {
        self.result
    }

    /// Call only after execution ownership has ended and termination is fixed.
    /// Result publication does not imply that VM or device cleanup succeeded.
    pub fn finish<const WAITS: usize>(
        &mut self,
        status: ExitStatus,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), ProcessError> {
        if self.task.is_none() {
            return Err(ProcessError::Unbound);
        }
        if self.result.is_some() {
            return Err(ProcessError::AlreadyExited);
        }
        self.result = Some(status);
        self.prune(waits);
        for observer in &mut self.observers {
            if let Some(observer) = observer.take() {
                let _ = waits.complete(observer.wait, Completion::ProcessExited(status));
            }
        }
        Ok(())
    }

    pub fn wait<const WAITS: usize>(
        &mut self,
        observer: TaskId,
        wait: WaitId,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        let target = self.task.ok_or(IpcError::InvalidWait)?;
        if observer == target {
            return Err(IpcError::PermissionDenied);
        }
        if !valid_wait(waits, observer, wait) {
            return Err(IpcError::InvalidWait);
        }
        self.prune(waits);
        if self
            .observers
            .iter()
            .flatten()
            .any(|entry| entry.task == observer || entry.wait == wait)
        {
            return Err(IpcError::AlreadyWaiting);
        }
        if let Some(status) = self.result {
            let _ = waits.complete(wait, Completion::ProcessExited(status));
        } else {
            let index = self
                .observers
                .vacant(Option::is_none, || None)
                .map_err(|_| IpcError::Capacity)?;
            self.observers[index] = Some(Observer {
                task: observer,
                wait,
            });
        }
        Ok(())
    }

    pub fn cancel<const WAITS: usize>(
        &mut self,
        wait: WaitId,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        self.cancel_with(wait, IpcError::Cancelled, waits)
    }

    pub fn timeout<const WAITS: usize>(
        &mut self,
        wait: WaitId,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        self.cancel_with(wait, IpcError::TimedOut, waits)
    }

    /// Observer death ends its registration, without changing the target.
    pub fn task_died<const WAITS: usize>(
        &mut self,
        task: TaskId,
        waits: &mut WaitTable<Completion, WAITS>,
    ) {
        self.prune(waits);
        for observer in &mut self.observers {
            if observer.is_some_and(|observer| observer.task == task) {
                let wait = observer.take().unwrap().wait;
                fail(waits, wait, IpcError::PeerClosed);
            }
        }
    }

    fn cancel_with<const WAITS: usize>(
        &mut self,
        wait: WaitId,
        error: IpcError,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        self.prune(waits);
        let slot = self
            .observers
            .iter_mut()
            .find(|observer| observer.is_some_and(|observer| observer.wait == wait))
            .ok_or(IpcError::InvalidWait)?;
        *slot = None;
        fail(waits, wait, error);
        Ok(())
    }

    fn prune<const WAITS: usize>(&mut self, waits: &WaitTable<Completion, WAITS>) {
        for observer in &mut self.observers {
            if observer.is_some_and(|observer| !valid_wait(waits, observer.task, observer.wait)) {
                *observer = None;
            }
        }
    }
}

fn valid_wait<const WAITS: usize>(
    waits: &WaitTable<Completion, WAITS>,
    observer: TaskId,
    wait: WaitId,
) -> bool {
    waits.task(wait) == Ok(observer) && waits.is_pending(wait)
}

fn fail<const WAITS: usize>(
    waits: &mut WaitTable<Completion, WAITS>,
    wait: WaitId,
    error: IpcError,
) {
    let _ = waits.complete(
        wait,
        Completion::Failed {
            error,
            delivery: Delivery::NotDelivered,
        },
    );
}
