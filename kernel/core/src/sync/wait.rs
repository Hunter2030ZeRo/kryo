use crate::storage::Slots;
use crate::task::TaskId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaitId(u64);

impl WaitId {
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
pub enum WaitError {
    Capacity,
    AlreadyWaiting,
    InvalidId,
    AlreadyParked,
    AlreadyCompleted,
    NotReady,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParkResult {
    Blocked,
    Ready,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Parking,
    Blocked,
    Ready,
}

struct Wait<T> {
    task: TaskId,
    phase: Phase,
    completion: Option<T>,
    deadline: Option<u64>,
}

struct Slot<T> {
    generation: u32,
    retired: bool,
    wait: Option<Wait<T>>,
}

/// Serialized wait ownership; IDs are interpreted within this table.
///
/// The caller retains the task's resources and serializes queue/task changes
/// with these operations. park follows CPU-context saving and execution handoff.
/// Before abandoning a dead task's wait, detach it from every external queue.
/// Retain this table while any external queue still contains one of its IDs.
/// SMP users need an outer synchronization and execution-ownership protocol.
pub struct WaitTable<T, const WAITS: usize> {
    slots: Slots<Slot<T>, WAITS>,
}

impl<T, const WAITS: usize> Default for WaitTable<T, WAITS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const WAITS: usize> WaitTable<T, WAITS> {
    pub const fn new() -> Self {
        Self {
            slots: Slots::new(
                [const {
                    Slot {
                        generation: 1,
                        retired: false,
                        wait: None,
                    }
                }; WAITS],
            ),
        }
    }

    pub const fn growing() -> Self {
        Self::limited(u32::MAX as usize)
    }

    pub const fn limited(limit: usize) -> Self {
        Self {
            slots: Slots::limited(limit),
        }
    }

    pub fn begin(&mut self, task: TaskId) -> Result<WaitId, WaitError> {
        self.begin_with_deadline(task, None)
    }

    /// Registers an absolute deadline in the runtime's monotonic nanoseconds.
    /// Expiry selection does not complete the wait or detach its object queues.
    pub fn begin_until(&mut self, task: TaskId, deadline: u64) -> Result<WaitId, WaitError> {
        self.begin_with_deadline(task, Some(deadline))
    }

    fn begin_with_deadline(
        &mut self,
        task: TaskId,
        deadline: Option<u64>,
    ) -> Result<WaitId, WaitError> {
        if self
            .slots
            .iter()
            .any(|slot| slot.wait.as_ref().is_some_and(|wait| wait.task == task))
        {
            return Err(WaitError::AlreadyWaiting);
        }
        let index = self
            .slots
            .vacant(
                |slot| !slot.retired && slot.wait.is_none(),
                || Slot {
                    generation: 1,
                    retired: false,
                    wait: None,
                },
            )
            .map_err(|_| WaitError::Capacity)?;
        let slot = &mut self.slots[index];
        let id = WaitId::new(index, slot.generation).ok_or(WaitError::Capacity)?;
        slot.wait = Some(Wait {
            task,
            phase: Phase::Parking,
            completion: None,
            deadline,
        });
        Ok(id)
    }

    pub fn task(&self, id: WaitId) -> Result<TaskId, WaitError> {
        Ok(self.slot(id)?.wait.as_ref().unwrap().task)
    }

    pub fn live_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.wait.is_some()).count()
    }

    /// False after any completion wins, even before Parking has ended.
    pub fn is_pending(&self, id: WaitId) -> bool {
        self.slot(id).is_ok_and(|slot| {
            let wait = slot.wait.as_ref().unwrap();
            wait.completion.is_none() && wait.phase != Phase::Ready
        })
    }

    pub fn next_deadline(&self) -> Option<u64> {
        self.pending_deadlines().map(|(_, deadline)| deadline).min()
    }

    /// The caller must detach object queues before completing this wait.
    /// Equal deadlines are selected in slot order; Parking waits remain hidden
    /// from scheduler publication even if timeout completes them here.
    pub fn next_expired(&self, now: u64) -> Option<WaitId> {
        self.pending_deadlines()
            .filter(|(_, deadline)| *deadline <= now)
            .min_by_key(|(id, deadline)| (*deadline, id.as_raw() as u32))
            .map(|(id, _)| id)
    }

    fn pending_deadlines(&self) -> impl Iterator<Item = (WaitId, u64)> + '_ {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            let wait = slot.wait.as_ref()?;
            if wait.completion.is_some() || wait.phase == Phase::Ready {
                return None;
            }
            Some((WaitId::new(index, slot.generation)?, wait.deadline?))
        })
    }

    /// Call only after the CPU has relinquished execution ownership.
    pub fn park(&mut self, id: WaitId) -> Result<ParkResult, WaitError> {
        let wait = self.slot_mut(id)?.wait.as_mut().unwrap();
        match wait.phase {
            Phase::Parking => {
                if wait.completion.is_some() {
                    wait.phase = Phase::Ready;
                    Ok(ParkResult::Ready)
                } else {
                    wait.phase = Phase::Blocked;
                    Ok(ParkResult::Blocked)
                }
            }
            Phase::Blocked => Err(WaitError::AlreadyParked),
            Phase::Ready => Ok(ParkResult::Ready),
        }
    }

    /// Records one completion. A task still Parking cannot be woken yet.
    /// The caller coordinates completion delivery with a single scheduler wakeup.
    /// Rejected completions are dropped; callers retain any external resources.
    pub fn complete(&mut self, id: WaitId, value: T) -> Result<Option<TaskId>, WaitError> {
        let wait = self.slot_mut(id)?.wait.as_mut().unwrap();
        if wait.completion.is_some() {
            return Err(WaitError::AlreadyCompleted);
        }
        wait.completion = Some(value);
        wait.deadline = None;
        if wait.phase == Phase::Blocked {
            wait.phase = Phase::Ready;
            Ok(Some(wait.task))
        } else {
            Ok(None)
        }
    }

    pub fn take(&mut self, id: WaitId) -> Result<T, WaitError> {
        let slot = self.slot_mut(id)?;
        let wait = slot.wait.as_mut().unwrap();
        if wait.phase != Phase::Ready {
            return Err(WaitError::NotReady);
        }
        let completion = wait
            .completion
            .take()
            .expect("Ready wait completion invariant");
        release_slot(slot);
        Ok(completion)
    }

    pub fn next_ready(&self) -> Option<(WaitId, TaskId)> {
        self.slots.iter().enumerate().find_map(|(index, slot)| {
            let wait = slot.wait.as_ref()?;
            if wait.phase != Phase::Ready {
                return None;
            }
            Some((WaitId::new(index, slot.generation)?, wait.task))
        })
    }

    pub fn ready(&self) -> impl Iterator<Item = (WaitId, TaskId, &T)> {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            let wait = slot.wait.as_ref()?;
            (wait.phase == Phase::Ready).then(|| {
                (
                    WaitId::new(index, slot.generation).unwrap(),
                    wait.task,
                    wait.completion
                        .as_ref()
                        .expect("Ready wait completion invariant"),
                )
            })
        })
    }

    /// Caller has detached this wait from queues and ended task execution.
    pub fn abandon(&mut self, id: WaitId) -> Result<Option<T>, WaitError> {
        let slot = self.slot_mut(id)?;
        let completion = slot.wait.as_mut().unwrap().completion.take();
        release_slot(slot);
        Ok(completion)
    }

    fn slot(&self, id: WaitId) -> Result<&Slot<T>, WaitError> {
        let (index, generation) = id.parts().ok_or(WaitError::InvalidId)?;
        let slot = self.slots.get(index).ok_or(WaitError::InvalidId)?;
        if slot.retired || slot.generation != generation || slot.wait.is_none() {
            return Err(WaitError::InvalidId);
        }
        Ok(slot)
    }

    fn slot_mut(&mut self, id: WaitId) -> Result<&mut Slot<T>, WaitError> {
        let (index, generation) = id.parts().ok_or(WaitError::InvalidId)?;
        let slot = self.slots.get_mut(index).ok_or(WaitError::InvalidId)?;
        if slot.retired || slot.generation != generation || slot.wait.is_none() {
            return Err(WaitError::InvalidId);
        }
        Ok(slot)
    }
}

fn release_slot<T>(slot: &mut Slot<T>) {
    slot.wait = None;
    if slot.generation == u32::MAX {
        slot.retired = true;
    } else {
        slot.generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::TaskTable;

    #[test]
    fn generation_wrap_retires_wait_slots() {
        let mut tasks = TaskTable::<(), 1>::new();
        let task = tasks.spawn(()).unwrap();
        let mut waits = WaitTable::<u8, 1>::new();
        waits.slots[0].generation = u32::MAX;
        let id = waits.begin(task).unwrap();
        assert_eq!(id.as_raw() >> 32, u64::from(u32::MAX));
        waits.abandon(id).unwrap();
        assert_eq!(waits.begin(task), Err(WaitError::Capacity));
        assert_eq!(waits.complete(id, 1), Err(WaitError::InvalidId));
    }
}
