use alloc::collections::VecDeque;

use super::WaitId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordWaitError {
    Capacity,
    DuplicateWait,
}

/// Serialized word queues sharing the runtime's persistent wait table.
/// Keys identify retained mapping lifetimes, not bare user addresses.
pub struct WordWaiters<K> {
    queued: VecDeque<(K, WaitId)>,
}

impl<K: Copy + Eq> WordWaiters<K> {
    pub const fn new() -> Self {
        Self {
            queued: VecDeque::new(),
        }
    }

    /// Reserve total queue capacity before charging or publishing another wait.
    pub fn prepare(&mut self, minimum: usize) -> Result<(), WordWaitError> {
        if minimum > self.queued.capacity() {
            self.queued
                .try_reserve_exact(minimum - self.queued.len())
                .map_err(|_| WordWaitError::Capacity)?;
        }
        Ok(())
    }

    /// Insert without allocating. The caller validates the pending WaitId.
    pub fn push(&mut self, key: K, wait: WaitId) -> Result<(), WordWaitError> {
        if self.queued.iter().any(|(_, queued)| *queued == wait) {
            return Err(WordWaitError::DuplicateWait);
        }
        if self.queued.len() == self.queued.capacity() {
            return Err(WordWaitError::Capacity);
        }
        self.queued.push_back((key, wait));
        Ok(())
    }

    /// Wake up to maximum matching waiters in admission order, with no latch.
    pub fn wake(&mut self, key: K, maximum: usize, complete: impl FnMut(WaitId)) -> usize {
        self.extract(|queued, _| queued == key, maximum, complete)
    }

    pub fn remove(&mut self, wait: WaitId) -> bool {
        self.extract(|_, queued| queued == wait, 1, |_| {}) != 0
    }

    /// Detach invalidated mappings before completing their waits.
    pub fn remove_matching(
        &mut self,
        mut matches: impl FnMut(K) -> bool,
        complete: impl FnMut(WaitId),
    ) -> usize {
        self.extract(|key, _| matches(key), usize::MAX, complete)
    }

    pub fn len(&self) -> usize {
        self.queued.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queued.is_empty()
    }

    fn extract(
        &mut self,
        mut matches: impl FnMut(K, WaitId) -> bool,
        maximum: usize,
        mut complete: impl FnMut(WaitId),
    ) -> usize {
        if maximum == 0 {
            return 0;
        }
        let mut removed = 0;
        for _ in 0..self.queued.len() {
            let (key, wait) = self.queued.pop_front().unwrap();
            if removed < maximum && matches(key, wait) {
                // Pop precedes completion; every remaining record reuses its
                // vacated capacity and preserves the original FIFO order.
                removed += 1;
                complete(wait);
            } else {
                self.queued.push_back((key, wait));
            }
        }
        removed
    }
}

impl<K: Copy + Eq> Default for WordWaiters<K> {
    fn default() -> Self {
        Self::new()
    }
}
