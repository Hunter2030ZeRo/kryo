use super::ExitStatus;
use crate::storage::Slots;
use crate::task::TaskId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessId(u64);

impl ProcessId {
    pub const fn as_raw(self) -> u64 {
        self.0
    }

    fn new(index: usize, generation: u32) -> Option<Self> {
        let slot = u32::try_from(index).ok()?.checked_add(1)?;
        Some(Self((u64::from(generation) << 32) | u64::from(slot)))
    }

    fn parts(self) -> Option<(usize, u32)> {
        let index = (self.0 as u32).checked_sub(1)? as usize;
        let generation = (self.0 >> 32) as u32;
        (generation != 0).then_some((index, generation))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessState {
    Active,
    Exiting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessTableError {
    Capacity,
    InvalidId,
    DuplicateThread,
    ThreadCapacity,
    Exiting,
    InvalidThread,
    ThreadNotEnded,
    HasThreads,
    NotExiting,
}

pub struct ProcessInsertError<P> {
    pub error: ProcessTableError,
    pub resources: P,
}

impl<P> core::fmt::Debug for ProcessInsertError<P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProcessReapError<E> {
    Process(ProcessTableError),
    Cleanup(E),
}

#[derive(Clone, Copy)]
struct Thread {
    task: TaskId,
    ended: bool,
}

struct Process<P, const THREADS: usize> {
    resources: P,
    threads: Slots<Option<Thread>, THREADS>,
    had_threads: bool,
    exit: Option<ExitStatus>,
}

struct Slot<P, const THREADS: usize> {
    generation: u32,
    retired: bool,
    process: Option<Process<P, THREADS>>,
}

/// Owns process resources independently of thread execution and cleanup.
///
/// Keep this table and the associated TaskTable for their full registry lifetime;
/// IDs are table-local. Serialize changes with task execution handoff. Attach
/// before the scheduler can dispatch a thread, mark it ended after its CPU releases
/// execution, and detach only after its private resources retire successfully.
/// Detaching the last thread does not release process resources: reap does.
pub struct ProcessTable<P, const PROCESSES: usize, const THREADS: usize> {
    slots: Slots<Slot<P, THREADS>, PROCESSES>,
    thread_limit: usize,
}

impl<P, const PROCESSES: usize, const THREADS: usize> Drop for ProcessTable<P, PROCESSES, THREADS> {
    fn drop(&mut self) {
        for slot in &mut self.slots {
            if let Some(process) = slot.process.take() {
                core::mem::forget(process.resources);
            }
        }
    }
}

impl<P, const PROCESSES: usize, const THREADS: usize> Default
    for ProcessTable<P, PROCESSES, THREADS>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<P, const PROCESSES: usize, const THREADS: usize> ProcessTable<P, PROCESSES, THREADS> {
    pub const fn new() -> Self {
        Self {
            slots: Slots::new(
                [const {
                    Slot {
                        generation: 1,
                        retired: false,
                        process: None,
                    }
                }; PROCESSES],
            ),
            thread_limit: THREADS,
        }
    }

    pub const fn growing() -> Self {
        Self::limited(u32::MAX as usize, usize::MAX)
    }

    pub const fn limited(processes: usize, threads: usize) -> Self {
        Self {
            slots: Slots::limited(processes),
            thread_limit: threads,
        }
    }

    pub fn reserve_slot(&mut self) -> Result<(), ProcessTableError> {
        self.slots
            .vacant(
                |slot| !slot.retired && slot.process.is_none(),
                || Slot {
                    generation: 1,
                    retired: false,
                    process: None,
                },
            )
            .map(|_| ())
            .map_err(|_| ProcessTableError::Capacity)
    }

    /// Reserve membership before the caller publishes a runnable thread.
    pub fn reserve_thread(&mut self, id: ProcessId) -> Result<(), ProcessTableError> {
        if self.state(id)? == ProcessState::Exiting {
            return Err(ProcessTableError::Exiting);
        }
        self.process_mut(id)?
            .threads
            .vacant(Option::is_none, || None)
            .map(|_| ())
            .map_err(|_| ProcessTableError::ThreadCapacity)
    }

    /// The caller prepares the initial thread before publishing this process.
    pub fn insert(&mut self, resources: P) -> Result<ProcessId, ProcessInsertError<P>> {
        if self.reserve_slot().is_err() {
            return Err(ProcessInsertError {
                error: ProcessTableError::Capacity,
                resources,
            });
        }
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| !slot.retired && slot.process.is_none())
        else {
            return Err(ProcessInsertError {
                error: ProcessTableError::Capacity,
                resources,
            });
        };
        let Some(id) = ProcessId::new(index, self.slots[index].generation) else {
            return Err(ProcessInsertError {
                error: ProcessTableError::Capacity,
                resources,
            });
        };
        self.slots[index].process = Some(Process {
            resources,
            threads: if self.slots.is_growing() {
                Slots::limited(self.thread_limit)
            } else {
                Slots::new([None; THREADS])
            },
            had_threads: false,
            exit: None,
        });
        Ok(id)
    }

    pub fn resources_mut(&mut self, id: ProcessId) -> Result<&mut P, ProcessTableError> {
        Ok(&mut self.process_mut(id)?.resources)
    }

    pub fn state(&self, id: ProcessId) -> Result<ProcessState, ProcessTableError> {
        Ok(if self.process(id)?.exit.is_some() {
            ProcessState::Exiting
        } else {
            ProcessState::Active
        })
    }

    pub fn ids(&self) -> impl Iterator<Item = ProcessId> + '_ {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            slot.process.as_ref()?;
            ProcessId::new(index, slot.generation)
        })
    }

    pub fn live_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.process.is_some())
            .count()
    }

    /// Includes ended threads whose private cleanup has not yet succeeded.
    pub fn thread_count(&self, id: ProcessId) -> Result<usize, ProcessTableError> {
        Ok(self.process(id)?.threads.iter().flatten().count())
    }

    pub fn threads(
        &self,
        id: ProcessId,
    ) -> Result<impl Iterator<Item = TaskId> + '_, ProcessTableError> {
        Ok(self
            .process(id)?
            .threads
            .iter()
            .flatten()
            .map(|thread| thread.task))
    }

    pub fn process_for(&self, task: TaskId) -> Option<ProcessId> {
        self.slots.iter().enumerate().find_map(|(index, slot)| {
            slot.process
                .as_ref()?
                .threads
                .iter()
                .flatten()
                .find(|thread| thread.task == task)?;
            ProcessId::new(index, slot.generation)
        })
    }

    pub fn attach(&mut self, id: ProcessId, task: TaskId) -> Result<(), ProcessTableError> {
        if self.state(id)? == ProcessState::Exiting {
            return Err(ProcessTableError::Exiting);
        }
        if self.process_for(task).is_some() {
            return Err(ProcessTableError::DuplicateThread);
        }
        self.reserve_thread(id)?;
        let process = self.process_mut(id)?;
        let index = process.threads.iter().position(Option::is_none).unwrap();
        process.threads[index] = Some(Thread { task, ended: false });
        process.had_threads = true;
        Ok(())
    }

    /// Fixes the first process-wide reason and rejects further thread attachment.
    /// The caller must stop every member; this does not end CPU execution itself.
    pub fn request_exit(
        &mut self,
        id: ProcessId,
        status: ExitStatus,
    ) -> Result<(), ProcessTableError> {
        let process = self.process_mut(id)?;
        process.exit.get_or_insert(status);
        Ok(())
    }

    /// Marks one execution handoff complete; repeated reports do not change status.
    /// If no process-wide exit was requested, the last thread's status wins.
    pub fn end_thread(
        &mut self,
        id: ProcessId,
        task: TaskId,
        status: ExitStatus,
    ) -> Result<Option<ExitStatus>, ProcessTableError> {
        let process = self.process_mut(id)?;
        let thread = process
            .threads
            .iter_mut()
            .flatten()
            .find(|thread| thread.task == task)
            .ok_or(ProcessTableError::InvalidThread)?;
        if !thread.ended {
            thread.ended = true;
            if process.threads.iter().flatten().all(|thread| thread.ended) {
                process.exit.get_or_insert(status);
            }
        }
        Ok(terminal(process))
    }

    pub fn terminal(&self, id: ProcessId) -> Result<Option<ExitStatus>, ProcessTableError> {
        Ok(terminal(self.process(id)?))
    }

    /// The caller has successfully retired this ended thread's private resources.
    pub fn detach(&mut self, id: ProcessId, task: TaskId) -> Result<(), ProcessTableError> {
        let process = self.process_mut(id)?;
        let slot = process
            .threads
            .iter_mut()
            .find(|slot| slot.is_some_and(|thread| thread.task == task))
            .ok_or(ProcessTableError::InvalidThread)?;
        if !slot.as_ref().unwrap().ended {
            return Err(ProcessTableError::ThreadNotEnded);
        }
        *slot = None;
        Ok(())
    }

    pub fn reap<E>(
        &mut self,
        id: ProcessId,
        cleanup: impl FnOnce(&mut P) -> Result<(), E>,
    ) -> Result<P, ProcessReapError<E>> {
        let process = self.process(id).map_err(ProcessReapError::Process)?;
        if process.exit.is_none() {
            return Err(ProcessReapError::Process(ProcessTableError::NotExiting));
        }
        if process.threads.iter().any(Option::is_some) {
            return Err(ProcessReapError::Process(ProcessTableError::HasThreads));
        }
        self.cleanup(id, cleanup)
    }

    /// Rolls back an unpublished process that has no thread memberships.
    pub fn abort<E>(
        &mut self,
        id: ProcessId,
        cleanup: impl FnOnce(&mut P) -> Result<(), E>,
    ) -> Result<P, ProcessReapError<E>> {
        let process = self.process(id).map_err(ProcessReapError::Process)?;
        if process.exit.is_some() {
            return Err(ProcessReapError::Process(ProcessTableError::Exiting));
        }
        if process.threads.iter().any(Option::is_some) {
            return Err(ProcessReapError::Process(ProcessTableError::HasThreads));
        }
        self.cleanup(id, cleanup)
    }

    fn cleanup<E>(
        &mut self,
        id: ProcessId,
        cleanup: impl FnOnce(&mut P) -> Result<(), E>,
    ) -> Result<P, ProcessReapError<E>> {
        cleanup(
            &mut self
                .process_mut(id)
                .map_err(ProcessReapError::Process)?
                .resources,
        )
        .map_err(ProcessReapError::Cleanup)?;
        let (index, _) = id
            .parts()
            .ok_or(ProcessReapError::Process(ProcessTableError::InvalidId))?;
        let slot = &mut self.slots[index];
        let process = slot.process.take().unwrap();
        if slot.generation == u32::MAX {
            slot.retired = true;
        } else {
            slot.generation += 1;
        }
        Ok(process.resources)
    }

    fn process(&self, id: ProcessId) -> Result<&Process<P, THREADS>, ProcessTableError> {
        let (index, generation) = id.parts().ok_or(ProcessTableError::InvalidId)?;
        let slot = self.slots.get(index).ok_or(ProcessTableError::InvalidId)?;
        if slot.retired || slot.generation != generation {
            return Err(ProcessTableError::InvalidId);
        }
        slot.process.as_ref().ok_or(ProcessTableError::InvalidId)
    }

    fn process_mut(
        &mut self,
        id: ProcessId,
    ) -> Result<&mut Process<P, THREADS>, ProcessTableError> {
        let (index, generation) = id.parts().ok_or(ProcessTableError::InvalidId)?;
        let slot = self
            .slots
            .get_mut(index)
            .ok_or(ProcessTableError::InvalidId)?;
        if slot.retired || slot.generation != generation {
            return Err(ProcessTableError::InvalidId);
        }
        slot.process.as_mut().ok_or(ProcessTableError::InvalidId)
    }
}

fn terminal<P, const THREADS: usize>(process: &Process<P, THREADS>) -> Option<ExitStatus> {
    if process.had_threads && process.threads.iter().flatten().all(|thread| thread.ended) {
        process.exit
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_wrap_retires_the_process_slot() {
        let mut processes = ProcessTable::<(), 1, 1>::new();
        processes.slots[0].generation = u32::MAX;
        let old = processes.insert(()).unwrap();
        assert_eq!(old.as_raw() >> 32, u64::from(u32::MAX));
        processes.abort(old, |_| Ok::<_, ()>(())).unwrap();
        assert_eq!(
            processes.insert(()).unwrap_err().error,
            ProcessTableError::Capacity
        );
        assert_eq!(processes.state(old), Err(ProcessTableError::InvalidId));
    }
}
