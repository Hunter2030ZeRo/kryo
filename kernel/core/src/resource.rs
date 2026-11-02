use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub shared_pages: usize,
    pub handles: usize,
    pub waits: usize,
    pub threads: usize,
    pub mappings: usize,
    pub tables: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Resource {
    SharedPages = 0,
    Handles = 1,
    Waits = 2,
    Threads = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetError {
    Capacity,
    InvalidAmount,
}

#[derive(Debug)]
struct Account {
    limits: Limits,
    used: [AtomicUsize; 4],
}

/// Shared resource accounting independent of a process's execution lifetime.
#[derive(Clone, Debug)]
pub struct Budget {
    account: Arc<Account>,
}

impl Budget {
    /// Allocate the account during trusted startup. Charging an existing account
    /// only updates its counter and clones its shared owner.
    pub fn new(limits: Limits) -> Self {
        Self {
            account: Arc::new(Account {
                limits,
                used: [const { AtomicUsize::new(0) }; 4],
            }),
        }
    }

    pub fn limits(&self) -> Limits {
        self.account.limits
    }

    pub fn limit(&self, resource: Resource) -> usize {
        match resource {
            Resource::SharedPages => self.account.limits.shared_pages,
            Resource::Handles => self.account.limits.handles,
            Resource::Waits => self.account.limits.waits,
            Resource::Threads => self.account.limits.threads,
        }
    }

    pub fn count(&self, resource: Resource) -> usize {
        self.account.used[resource as usize].load(Ordering::Relaxed)
    }

    pub fn try_charge(&self, resource: Resource, amount: usize) -> Result<Charge, BudgetError> {
        if amount == 0 {
            return Err(BudgetError::InvalidAmount);
        }
        let limit = self.limit(resource);
        self.account.used[resource as usize]
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(amount).filter(|next| *next <= limit)
            })
            .map_err(|_| BudgetError::Capacity)?;
        Ok(Charge {
            account: Arc::clone(&self.account),
            resource,
            amount,
        })
    }
}

/// Keep the charge with the actual resource owner, including deferred cleanup.
#[must_use]
#[derive(Debug)]
pub struct Charge {
    account: Arc<Account>,
    resource: Resource,
    amount: usize,
}

impl Charge {
    pub const fn resource(&self) -> Resource {
        self.resource
    }

    pub const fn amount(&self) -> usize {
        self.amount
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        let old =
            self.account.used[self.resource as usize].fetch_sub(self.amount, Ordering::Relaxed);
        debug_assert!(old >= self.amount);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_charge_owns_account_after_all_budget_handles_drop() {
        let budget = Budget::new(Limits {
            shared_pages: 1,
            handles: 0,
            waits: 0,
            threads: 0,
            mappings: 0,
            tables: 0,
        });
        let weak = Arc::downgrade(&budget.account);
        let charge = budget.try_charge(Resource::SharedPages, 1).unwrap();
        drop(budget);
        assert_eq!(weak.strong_count(), 1);
        assert_eq!(weak.upgrade().unwrap().used[0].load(Ordering::Relaxed), 1);
        drop(charge);
        assert!(weak.upgrade().is_none());
    }
}
