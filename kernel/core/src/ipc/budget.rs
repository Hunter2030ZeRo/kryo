use super::IpcError;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

struct Budget {
    limit: usize,
    live: AtomicUsize,
}

/// A shared limit on live channel payloads, independent of factory handles.
#[derive(Clone)]
pub struct ChannelBudget {
    budget: Arc<Budget>,
}

impl ChannelBudget {
    /// Allocates the account during trusted boot setup. User creation only
    /// reserves this existing account and never allocates another account.
    pub fn new(limit: usize) -> Self {
        Self {
            budget: Arc::new(Budget {
                limit,
                live: AtomicUsize::new(0),
            }),
        }
    }

    pub fn try_reserve(&self) -> Result<ChannelLease, IpcError> {
        self.budget
            .live
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |live| {
                (live < self.budget.limit).then(|| live + 1)
            })
            .map_err(|_| IpcError::Capacity)?;
        Ok(ChannelLease {
            budget: Arc::clone(&self.budget),
        })
    }

    pub fn live_count(&self) -> usize {
        self.budget.live.load(Ordering::Relaxed)
    }

    pub fn limit(&self) -> usize {
        self.budget.limit
    }
}

/// Retain this charge until the channel payload's final owner is released.
/// Closing a channel stops communication but does not release its charge.
#[must_use]
pub struct ChannelLease {
    budget: Arc<Budget>,
}

impl ChannelLease {
    pub(super) fn try_reserve(&self) -> Result<Self, IpcError> {
        ChannelBudget {
            budget: Arc::clone(&self.budget),
        }
        .try_reserve()
    }
}

impl Drop for ChannelLease {
    fn drop(&mut self) {
        let previous = self.budget.live.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(previous > 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_keeps_account_alive_after_all_factory_owners_drop() {
        let budget = ChannelBudget::new(1);
        let weak = Arc::downgrade(&budget.budget);
        let lease = budget.try_reserve().unwrap();
        drop(budget);
        assert_eq!(weak.strong_count(), 1);
        assert_eq!(weak.upgrade().unwrap().live.load(Ordering::Relaxed), 1);
        drop(lease);
        assert!(weak.upgrade().is_none());
    }
}
