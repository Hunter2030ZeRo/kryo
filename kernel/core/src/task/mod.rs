use crate::sched::{ReturnReason, TaskState};
use crate::storage::Slots;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskId(u64);

impl TaskId {
    pub const fn as_raw(self) -> u64 {
        self.0
    }

    fn new(slot: usize, generation: u32) -> Option<Self> {
        let slot = u32::try_from(slot).ok()?.checked_add(1)?;
        Some(Self((u64::from(generation) << 32) | u64::from(slot)))
    }

    fn parts(self) -> Option<(usize, u32)> {
        let slot = (self.0 as u32).checked_sub(1)? as usize;
        let generation = (self.0 >> 32) as u32;
        (generation != 0).then_some((slot, generation))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskError {
    Capacity,
    InvalidId,
    InvalidCpu,
    AlreadyRunning,
    NotRunning,
    Running,
    NotBlocked,
    NotDying,
    Dying,
}

pub struct SpawnError<T> {
    pub error: TaskError,
    pub resources: T,
}

impl<T> core::fmt::Debug for SpawnError<T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReapError<E> {
    Task(TaskError),
    Cleanup(E),
}

struct Task<T> {
    resources: T,
    state: TaskState,
}

struct Slot<T> {
    generation: u32,
    retired: bool,
    task: Option<Task<T>>,
}

/// Serialized scheduling ownership, with at most one task running on each CPU.
/// CPU indices are local to this table. Hardware execution must end and saved
/// context publication must finish before returned_on releases an owner.
/// Resources retire only through successful reap; dropping the table retains them.
pub struct TaskTable<T, const TASKS: usize, const CPUS: usize = 1> {
    slots: Slots<Slot<T>, TASKS>,
    current: [Option<TaskId>; CPUS],
    cursor: usize,
}

impl<T, const TASKS: usize, const CPUS: usize> Drop for TaskTable<T, TASKS, CPUS> {
    fn drop(&mut self) {
        for slot in &mut self.slots {
            if let Some(task) = slot.task.take() {
                core::mem::forget(task.resources);
            }
        }
    }
}

impl<T, const TASKS: usize, const CPUS: usize> Default for TaskTable<T, TASKS, CPUS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const TASKS: usize, const CPUS: usize> TaskTable<T, TASKS, CPUS> {
    pub const fn new() -> Self {
        Self {
            slots: Slots::new(
                [const {
                    Slot {
                        generation: 1,
                        retired: false,
                        task: None,
                    }
                }; TASKS],
            ),
            current: [None; CPUS],
            cursor: 0,
        }
    }

    pub const fn growing() -> Self {
        Self::limited(u32::MAX as usize)
    }

    pub const fn limited(limit: usize) -> Self {
        Self {
            slots: Slots::limited(limit),
            current: [None; CPUS],
            cursor: 0,
        }
    }

    /// Prepare metadata before publishing any process membership or CPU owner.
    pub fn reserve_slot(&mut self) -> Result<(), TaskError> {
        self.slots
            .vacant(
                |slot| !slot.retired && slot.task.is_none(),
                || Slot {
                    generation: 1,
                    retired: false,
                    task: None,
                },
            )
            .map(|_| ())
            .map_err(|_| TaskError::Capacity)
    }

    pub fn spawn(&mut self, resources: T) -> Result<TaskId, SpawnError<T>> {
        self.insert(resources, TaskState::Runnable)
    }

    /// Retains a prepared thread without allowing the scheduler to dispatch it.
    /// Finish process attachment and capability publication before calling wake.
    pub fn spawn_suspended(&mut self, resources: T) -> Result<TaskId, SpawnError<T>> {
        self.insert(resources, TaskState::Blocked)
    }

    fn insert(&mut self, resources: T, state: TaskState) -> Result<TaskId, SpawnError<T>> {
        if self.reserve_slot().is_err() {
            return Err(SpawnError {
                error: TaskError::Capacity,
                resources,
            });
        }
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| !slot.retired && slot.task.is_none())
        else {
            return Err(SpawnError {
                error: TaskError::Capacity,
                resources,
            });
        };
        let Some(id) = TaskId::new(index, self.slots[index].generation) else {
            return Err(SpawnError {
                error: TaskError::Capacity,
                resources,
            });
        };
        self.slots[index].task = Some(Task { resources, state });
        Ok(id)
    }

    pub fn dispatch(&mut self) -> Result<Option<TaskId>, TaskError> {
        self.dispatch_on(0)
    }

    pub fn dispatch_on(&mut self, cpu: usize) -> Result<Option<TaskId>, TaskError> {
        if self
            .current
            .get(cpu)
            .ok_or(TaskError::InvalidCpu)?
            .is_some()
        {
            return Err(TaskError::AlreadyRunning);
        }
        for distance in 0..self.slots.len() {
            let index = (self.cursor + distance) % self.slots.len();
            let slot = &mut self.slots[index];
            if let Some(task) = &mut slot.task
                && task.state == TaskState::Runnable
            {
                let id = TaskId::new(index, slot.generation).ok_or(TaskError::Capacity)?;
                task.state = TaskState::Running;
                self.current[cpu] = Some(id);
                self.cursor = (index + 1) % self.slots.len();
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    pub fn current(&self) -> Option<TaskId> {
        self.current.first().copied().flatten()
    }

    pub fn current_on(&self, cpu: usize) -> Result<Option<TaskId>, TaskError> {
        self.current.get(cpu).copied().ok_or(TaskError::InvalidCpu)
    }

    pub fn has_running(&self) -> bool {
        self.current.iter().any(Option::is_some)
    }

    pub fn has_runnable(&self) -> bool {
        self.slots.iter().any(|slot| {
            slot.task
                .as_ref()
                .is_some_and(|task| task.state == TaskState::Runnable)
        })
    }

    pub fn ids(&self) -> impl Iterator<Item = TaskId> + '_ {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            slot.task.as_ref()?;
            TaskId::new(index, slot.generation)
        })
    }

    pub fn state(&self, id: TaskId) -> Result<TaskState, TaskError> {
        Ok(self.task(id)?.state)
    }

    /// Coordinator access does not itself quiesce hardware. CPUs must not retain
    /// Rust references into these resources while this exclusive borrow exists.
    pub fn resources(&self, id: TaskId) -> Result<&T, TaskError> {
        Ok(&self.task(id)?.resources)
    }

    pub fn resources_mut(&mut self, id: TaskId) -> Result<&mut T, TaskError> {
        Ok(&mut self.task_mut(id)?.resources)
    }

    /// Call after CPU execution has stopped and its context has been saved completely.
    pub fn returned(&mut self, id: TaskId, reason: ReturnReason) -> Result<TaskState, TaskError> {
        self.returned_on(0, id, reason)
    }

    /// Call after this CPU stopped execution and published the complete saved context.
    /// A return attributed to another CPU cannot release either execution owner.
    pub fn returned_on(
        &mut self,
        cpu: usize,
        id: TaskId,
        reason: ReturnReason,
    ) -> Result<TaskState, TaskError> {
        let current = self.current.get(cpu).ok_or(TaskError::InvalidCpu)?;
        if *current != Some(id) || self.task(id)?.state != TaskState::Running {
            return Err(TaskError::NotRunning);
        }
        // End execution ownership before making the saved task runnable or reclaimable.
        self.current[cpu] = None;
        let state = match reason {
            ReturnReason::Preempted | ReturnReason::Yielded => TaskState::Runnable,
            ReturnReason::Call => TaskState::Blocked,
            ReturnReason::Fault | ReturnReason::Exit => TaskState::Dying,
        };
        self.task_mut(id)?.state = state;
        Ok(state)
    }

    pub fn wake(&mut self, id: TaskId) -> Result<(), TaskError> {
        let task = self.task_mut(id)?;
        if task.state != TaskState::Blocked {
            return Err(TaskError::NotBlocked);
        }
        task.state = TaskState::Runnable;
        Ok(())
    }

    pub fn stop(&mut self, id: TaskId) -> Result<(), TaskError> {
        let task = self.task_mut(id)?;
        match task.state {
            TaskState::Running => Err(TaskError::Running),
            TaskState::Dying => Err(TaskError::Dying),
            TaskState::Runnable | TaskState::Blocked => {
                task.state = TaskState::Dying;
                Ok(())
            }
        }
    }

    pub fn reap<E>(
        &mut self,
        id: TaskId,
        cleanup: impl FnOnce(&mut T) -> Result<(), E>,
    ) -> Result<T, ReapError<E>> {
        if self.state(id).map_err(ReapError::Task)? != TaskState::Dying {
            return Err(ReapError::Task(TaskError::NotDying));
        }
        cleanup(self.resources_mut(id).map_err(ReapError::Task)?).map_err(ReapError::Cleanup)?;
        let (index, _) = id.parts().ok_or(ReapError::Task(TaskError::InvalidId))?;
        let slot = &mut self.slots[index];
        let task = slot.task.take().unwrap();
        if slot.generation == u32::MAX {
            slot.retired = true;
        } else {
            slot.generation += 1;
        }
        Ok(task.resources)
    }

    pub fn live_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.task.is_some()).count()
    }

    fn task(&self, id: TaskId) -> Result<&Task<T>, TaskError> {
        let (index, generation) = id.parts().ok_or(TaskError::InvalidId)?;
        let slot = self.slots.get(index).ok_or(TaskError::InvalidId)?;
        if slot.generation != generation || slot.retired {
            return Err(TaskError::InvalidId);
        }
        slot.task.as_ref().ok_or(TaskError::InvalidId)
    }

    fn task_mut(&mut self, id: TaskId) -> Result<&mut Task<T>, TaskError> {
        let (index, generation) = id.parts().ok_or(TaskError::InvalidId)?;
        let slot = self.slots.get_mut(index).ok_or(TaskError::InvalidId)?;
        if slot.generation != generation || slot.retired {
            return Err(TaskError::InvalidId);
        }
        slot.task.as_mut().ok_or(TaskError::InvalidId)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_wrap_retires_the_slot() {
        let mut tasks = TaskTable::<(), 1>::new();
        tasks.slots[0].generation = u32::MAX;
        let id = tasks.spawn(()).unwrap();
        tasks.stop(id).unwrap();
        tasks.reap(id, |_| Ok::<_, ()>(())).unwrap();
        assert_eq!(tasks.spawn(()).unwrap_err().error, TaskError::Capacity);
        assert_eq!(tasks.state(id), Err(TaskError::InvalidId));
    }
}
