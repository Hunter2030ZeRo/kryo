use kryo_interfaces::file::{LockMode, LockOptions, Status};

#[derive(Clone, Copy)]
struct Owner {
    label: u64,
    inode: u64,
    held: Option<LockMode>,
}

#[derive(Clone, Copy)]
pub struct Pending<T: Copy> {
    pub label: u64,
    pub id: u64,
    pub reply: T,
    inode: u64,
    mode: LockMode,
}

#[derive(Clone, Copy)]
pub struct Grant<T: Copy> {
    pub pending: Pending<T>,
    previous: Option<LockMode>,
}

pub struct Locks<T: Copy> {
    owners: Vec<Owner>,
    pending: Vec<Pending<T>>,
}

impl<T: Copy> Locks<T> {
    pub fn new() -> Self {
        Self {
            owners: Vec::new(),
            pending: Vec::new(),
        }
    }

    pub fn reserve_owner(&mut self) -> Result<(), Status> {
        self.owners.try_reserve(1).map_err(|_| Status::NoSpace)
    }

    pub fn add_owner(&mut self, label: u64, inode: u64) {
        self.owners.push(Owner {
            label,
            inode,
            held: None,
        });
    }

    fn owner(&self, label: u64) -> Option<usize> {
        self.owners.iter().position(|owner| owner.label == label)
    }

    fn conflicts(&self, label: u64, inode: u64, mode: LockMode) -> bool {
        self.owners.iter().any(|owner| {
            owner.label != label
                && owner.inode == inode
                && owner
                    .held
                    .is_some_and(|held| held == LockMode::Exclusive || mode == LockMode::Exclusive)
        })
    }

    pub fn cancel(&mut self, label: u64) -> Option<Pending<T>> {
        self.pending
            .iter()
            .position(|pending| pending.label == label)
            .map(|index| self.pending.remove(index))
    }

    pub fn remove_owner(&mut self, label: u64) -> Option<Pending<T>> {
        if let Some(index) = self.owner(label) {
            self.owners.swap_remove(index);
        }
        self.cancel(label)
    }

    pub fn request(
        &mut self,
        label: u64,
        options: LockOptions,
        id: u64,
        reply: T,
    ) -> Result<Option<Grant<T>>, Status> {
        let index = self.owner(label).ok_or(Status::BadFile)?;
        let owner = self.owners[index];
        if self.pending.iter().any(|pending| pending.label == label) {
            return Err(if options.nonblocking {
                Status::WouldBlock
            } else {
                Status::Deadlock
            });
        }
        let pending = Pending {
            label,
            inode: owner.inode,
            mode: options.mode,
            id,
            reply,
        };
        let queued = self
            .pending
            .iter()
            .any(|pending| pending.inode == owner.inode);
        if owner.held == Some(options.mode)
            || (owner.held == Some(LockMode::Exclusive) && options.mode == LockMode::Shared)
            || (!queued && !self.conflicts(label, owner.inode, options.mode))
        {
            self.owners[index].held = Some(options.mode);
            return Ok(Some(Grant {
                pending,
                previous: owner.held,
            }));
        }
        if options.nonblocking {
            return Err(Status::WouldBlock);
        }
        if owner.held == Some(LockMode::Shared) {
            return Err(Status::Deadlock);
        }
        self.pending.try_reserve(1).map_err(|_| Status::NoSpace)?;
        self.pending.push(pending);
        Ok(None)
    }

    pub fn unlock(&mut self, label: u64) -> Result<Option<Pending<T>>, Status> {
        let index = self.owner(label).ok_or(Status::BadFile)?;
        self.owners[index].held = None;
        Ok(self.cancel(label))
    }

    pub fn next_ready(&mut self) -> Option<Grant<T>> {
        for index in 0..self.pending.len() {
            let pending = self.pending[index];
            if self.pending[..index]
                .iter()
                .any(|earlier| earlier.inode == pending.inode)
                || self.conflicts(pending.label, pending.inode, pending.mode)
            {
                continue;
            }
            let owner = self.owner(pending.label)?;
            self.pending.remove(index);
            let previous = self.owners[owner].held.replace(pending.mode);
            return Some(Grant { pending, previous });
        }
        None
    }

    pub fn rollback(&mut self, grant: Grant<T>) {
        if let Some(index) = self.owner(grant.pending.label) {
            self.owners[index].held = grant.previous;
        }
    }

    pub fn pop_pending(&mut self) -> Option<Pending<T>> {
        if self.pending.is_empty() {
            None
        } else {
            Some(self.pending.remove(0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        locks: &mut Locks<u64>,
        label: u64,
        mode: LockMode,
        nonblocking: bool,
    ) -> Result<Option<Grant<u64>>, Status> {
        locks.request(
            label,
            LockOptions { mode, nonblocking },
            label + 100,
            label + 1000,
        )
    }

    fn owners(count: u64) -> Locks<u64> {
        let mut locks = Locks::new();
        for label in 1..=count {
            locks.reserve_owner().unwrap();
            locks.add_owner(label, 42);
        }
        locks
    }

    #[test]
    fn growable_owners_and_fifo_writer_fairness_have_no_fixture_open_limit() {
        let mut locks = owners(128);
        request(&mut locks, 1, LockMode::Shared, false).unwrap();
        assert!(
            request(&mut locks, 2, LockMode::Exclusive, false)
                .unwrap()
                .is_none()
        );
        for label in 3..=128 {
            assert!(
                request(&mut locks, label, LockMode::Shared, false)
                    .unwrap()
                    .is_none()
            );
        }
        assert!(locks.next_ready().is_none());
        locks.remove_owner(1);
        assert_eq!(locks.next_ready().unwrap().pending.label, 2);
        assert!(locks.next_ready().is_none());
        locks.unlock(2).unwrap();
        for label in 3..=128 {
            assert_eq!(locks.next_ready().unwrap().pending.label, label);
        }
        assert!(locks.next_ready().is_none());
    }

    #[test]
    fn upgrades_and_canceled_replies_preserve_prior_ownership() {
        let mut locks = owners(3);
        request(&mut locks, 1, LockMode::Shared, false).unwrap();
        request(&mut locks, 2, LockMode::Shared, false).unwrap();
        assert!(matches!(
            request(&mut locks, 1, LockMode::Exclusive, false),
            Err(Status::Deadlock)
        ));
        assert!(matches!(
            request(&mut locks, 1, LockMode::Exclusive, true),
            Err(Status::WouldBlock)
        ));
        assert!(
            request(&mut locks, 3, LockMode::Exclusive, false)
                .unwrap()
                .is_none()
        );
        locks.unlock(2).unwrap();
        assert!(locks.next_ready().is_none());
        locks.remove_owner(1);
        let tentative = locks.next_ready().unwrap();
        locks.rollback(tentative);
        assert_eq!(locks.owners[locks.owner(3).unwrap()].held, None);
        let exclusive = request(&mut locks, 3, LockMode::Exclusive, false)
            .unwrap()
            .unwrap();
        assert_eq!(exclusive.previous, None);
        let downgrade = request(&mut locks, 3, LockMode::Shared, false)
            .unwrap()
            .unwrap();
        locks.rollback(downgrade);
        assert_eq!(
            locks.owners[locks.owner(3).unwrap()].held,
            Some(LockMode::Exclusive)
        );
    }

    #[test]
    fn duplicate_pending_owner_is_rejected_and_close_returns_exact_token() {
        let mut locks = owners(3);
        request(&mut locks, 1, LockMode::Exclusive, false).unwrap();
        request(&mut locks, 2, LockMode::Exclusive, false).unwrap();
        request(&mut locks, 3, LockMode::Shared, false).unwrap();
        assert!(matches!(
            request(&mut locks, 2, LockMode::Shared, false),
            Err(Status::Deadlock)
        ));
        let canceled = locks.remove_owner(2).unwrap();
        assert_eq!((canceled.id, canceled.reply), (102, 1002));
        locks.remove_owner(1);
        assert_eq!(locks.next_ready().unwrap().pending.label, 3);
        assert!(locks.pop_pending().is_none());
    }
}
