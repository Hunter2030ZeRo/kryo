use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub shared_pages: usize,
    pub private_pages: usize,
    pub reserved_pages: usize,
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
    PrivatePages = 4,
    ReservedPages = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetError {
    Capacity,
    InvalidAmount,
}

#[derive(Debug)]
struct Account {
    limits: Limits,
    used: [AtomicUsize; 6],
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
                used: [const { AtomicUsize::new(0) }; 6],
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
            Resource::PrivatePages => self.account.limits.private_pages,
            Resource::ReservedPages => self.account.limits.reserved_pages,
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

    /// Refund only the units whose underlying resources have retired.
    pub fn refund(&mut self, amount: usize) -> Result<(), BudgetError> {
        if amount == 0 || amount > self.amount {
            return Err(BudgetError::InvalidAmount);
        }
        self.amount -= amount;
        let old = self.account.used[self.resource as usize].fetch_sub(amount, Ordering::Relaxed);
        debug_assert!(old >= amount);
        Ok(())
    }

    /// Combine ownership without charging the account again.
    pub fn merge(&mut self, mut other: Self) -> Result<(), Self> {
        if self.resource != other.resource || !Arc::ptr_eq(&self.account, &other.account) {
            return Err(other);
        }
        let Some(amount) = self.amount.checked_add(other.amount) else {
            return Err(other);
        };
        self.amount = amount;
        other.amount = 0;
        Ok(())
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
            private_pages: 0,
            reserved_pages: 0,
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

    #[test]
    fn merge_overflow_preserves_both_charges() {
        let budget = Budget::new(Limits {
            shared_pages: 0,
            private_pages: 2,
            reserved_pages: 0,
            handles: 0,
            waits: 0,
            threads: 0,
            mappings: 0,
            tables: 0,
        });
        let mut charge = budget.try_charge(Resource::PrivatePages, 1).unwrap();
        let extra = budget.try_charge(Resource::PrivatePages, 1).unwrap();
        // Real charges cannot overflow their shared account's total.
        charge.amount = usize::MAX;
        let returned = charge.merge(extra).unwrap_err();
        let amount = charge.amount;
        charge.amount = 1;
        assert_eq!(amount, usize::MAX);
        assert_eq!(returned.amount(), 1);
        assert_eq!(budget.count(Resource::PrivatePages), 2);
        drop((charge, returned));
        assert_eq!(budget.count(Resource::PrivatePages), 0);
    }
}
