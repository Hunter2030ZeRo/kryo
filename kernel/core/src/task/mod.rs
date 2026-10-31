use crate::sched::{ReturnReason, TaskState};

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

/// Resources are reclaimed only by successful reap; dropping the table leaks unreaped resources.
pub struct TaskTable<T, const TASKS: usize> {
    slots: [Slot<T>; TASKS],
    current: Option<TaskId>,
    cursor: usize,
}

impl<T, const TASKS: usize> Drop for TaskTable<T, TASKS> {
    fn drop(&mut self) {
        for slot in &mut self.slots {
            if let Some(task) = slot.task.take() {
                core::mem::forget(task.resources);
            }
        }
    }
}

impl<T, const TASKS: usize> Default for TaskTable<T, TASKS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const TASKS: usize> TaskTable<T, TASKS> {
    pub const fn new() -> Self {
        Self {
            slots: [const {
                Slot {
                    generation: 1,
                    retired: false,
                    task: None,
                }
            }; TASKS],
            current: None,
            cursor: 0,
        }
    }

    pub fn spawn(&mut self, resources: T) -> Result<TaskId, SpawnError<T>> {
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
        self.slots[index].task = Some(Task {
            resources,
            state: TaskState::Runnable,
        });
        Ok(id)
    }

    pub fn dispatch(&mut self) -> Result<Option<TaskId>, TaskError> {
        if self.current.is_some() {
            return Err(TaskError::AlreadyRunning);
        }
        for distance in 0..TASKS {
            let index = (self.cursor + distance) % TASKS;
            let slot = &mut self.slots[index];
            if let Some(task) = &mut slot.task
                && task.state == TaskState::Runnable
            {
                let id = TaskId::new(index, slot.generation).ok_or(TaskError::Capacity)?;
                task.state = TaskState::Running;
                self.current = Some(id);
                self.cursor = (index + 1) % TASKS;
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    pub fn current(&self) -> Option<TaskId> {
        self.current
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

    /// The exclusive borrow pins resources while the scheduler enters and saves the task.
    pub fn resources_mut(&mut self, id: TaskId) -> Result<&mut T, TaskError> {
        Ok(&mut self.task_mut(id)?.resources)
    }

    /// Call after CPU execution has stopped and its context has been saved completely.
    pub fn returned(&mut self, id: TaskId, reason: ReturnReason) -> Result<TaskState, TaskError> {
        if self.current != Some(id) || self.task(id)?.state != TaskState::Running {
            return Err(TaskError::NotRunning);
        }
        // End execution ownership before making the saved task runnable or reclaimable.
        self.current = None;
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
