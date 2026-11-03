use alloc::vec::Vec;

use super::{AddressSpace, FrameAllocator, VmBackend, VmError};
use crate::mm::frame::FRAME_SIZE;
use crate::mm::shared::SharedPage;
use crate::resource::{Budget, Charge, Resource};

const ARENA_START: u64 = 0x1_0000_0000;
const ARENA_END: u64 = 0x2000_0000_0000;

#[derive(Debug, PartialEq, Eq)]
pub enum PrivateError<E> {
    InvalidRange,
    Conflict,
    OutOfResources,
    Vm(VmError<E>),
    RetirementPending,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Live,
    Retiring,
}

#[derive(Clone, Copy)]
struct Commit {
    start: u64,
    end: u64,
    id: u64,
    state: State,
}

struct Region {
    start: u64,
    end: u64,
    id: u64,
    budget: Budget,
    _reserved: Charge,
    resident: Option<Charge>,
    commits: Vec<Commit>,
    closing: bool,
}

/// Process-local anonymous reservations. Range records describe reserved holes;
/// only committed pages consume frame owners in the associated address space.
pub struct PrivateVm {
    regions: Vec<Region>,
    next_region: Option<u64>,
    next_commit: Option<u64>,
    space: Option<u64>,
}

impl PrivateVm {
    pub const fn new() -> Self {
        Self {
            regions: Vec::new(),
            next_region: Some(1),
            next_commit: Some(1),
            space: None,
        }
    }

    pub fn reserve<
        F: FrameAllocator,
        B: VmBackend,
        const M: usize,
        const T: usize,
        S: SharedPage,
    >(
        &mut self,
        space: &AddressSpace<'_, F, B, M, T, S>,
        budget: &Budget,
        bytes: u64,
        alignment: u64,
    ) -> Result<u64, PrivateError<B::Error>> {
        self.check_space(space)?;
        if self.retirement_pending() {
            return Err(PrivateError::RetirementPending);
        }
        let pages = page_count(bytes).ok_or(PrivateError::InvalidRange)?;
        if alignment < FRAME_SIZE || !alignment.is_power_of_two() {
            return Err(PrivateError::InvalidRange);
        }
        let mut start = align_up(ARENA_START, alignment).ok_or(PrivateError::InvalidRange)?;
        loop {
            let end = start
                .checked_add(bytes)
                .filter(|end| *end <= ARENA_END)
                .ok_or(PrivateError::OutOfResources)?;
            if let Some(region) = self
                .regions
                .iter()
                .find(|region| start < region.end && region.start < end)
            {
                start = align_up(region.end, alignment).ok_or(PrivateError::OutOfResources)?;
                continue;
            }
            if let Some(last) = space.last_occupied(start, end) {
                start =
                    align_up(last + FRAME_SIZE, alignment).ok_or(PrivateError::OutOfResources)?;
                continue;
            }
            self.regions
                .try_reserve(1)
                .map_err(|_| PrivateError::OutOfResources)?;
            let reserved = budget
                .try_charge(Resource::ReservedPages, pages)
                .map_err(|_| PrivateError::OutOfResources)?;
            let id = take_id(&mut self.next_region).ok_or(PrivateError::OutOfResources)?;
            let index = self.regions.partition_point(|region| region.start < start);
            self.regions.insert(
                index,
                Region {
                    start,
                    end,
                    id,
                    budget: budget.clone(),
                    _reserved: reserved,
                    resident: None,
                    commits: Vec::new(),
                    closing: false,
                },
            );
            self.space = Some(space.identity());
            return Ok(start);
        }
    }

    pub fn commit<
        F: FrameAllocator,
        B: VmBackend,
        const M: usize,
        const T: usize,
        S: SharedPage,
    >(
        &mut self,
        space: &mut AddressSpace<'_, F, B, M, T, S>,
        address: u64,
        bytes: u64,
    ) -> Result<(), PrivateError<B::Error>> {
        self.check_space(space)?;
        if self.retirement_pending() {
            return Err(PrivateError::RetirementPending);
        }
        let (index, end, pages) = self
            .range(address, bytes)
            .ok_or(PrivateError::InvalidRange)?;
        space
            .ensure_unleased(address, bytes)
            .map_err(PrivateError::Vm)?;
        let region = &mut self.regions[index];
        if region.closing
            || region
                .commits
                .iter()
                .any(|part| address < part.end && part.start < end)
            || space.last_occupied(address, end).is_some()
        {
            return Err(PrivateError::Conflict);
        }
        region
            .commits
            .try_reserve(1)
            .map_err(|_| PrivateError::OutOfResources)?;
        let charge = region
            .budget
            .try_charge(Resource::PrivatePages, pages)
            .map_err(|_| PrivateError::OutOfResources)?;
        let id = take_id(&mut self.next_commit).ok_or(PrivateError::OutOfResources)?;
        merge_charge(&mut region.resident, charge);
        let part = region.commits.partition_point(|part| part.start < address);
        region.commits.insert(
            part,
            Commit {
                start: address,
                end,
                id,
                state: State::Retiring,
            },
        );
        match space.map_private_range(address, pages) {
            Ok(()) => {
                region.commits[part].state = State::Live;
                Ok(())
            }
            Err(error) => {
                // The failing map may own an unpublished frame. Count owners,
                // rather than PTEs, before returning any physical-memory credit.
                let owned = space.owned_pages_in_range(address, end);
                refund(region, pages - owned);
                let result = retire(region, space);
                if result.is_err()
                    || region
                        .commits
                        .iter()
                        .any(|part| part.state == State::Retiring)
                {
                    Err(PrivateError::RetirementPending)
                } else {
                    Err(PrivateError::Vm(error))
                }
            }
        }
    }

    pub fn decommit<
        F: FrameAllocator,
        B: VmBackend,
        const M: usize,
        const T: usize,
        S: SharedPage,
    >(
        &mut self,
        space: &mut AddressSpace<'_, F, B, M, T, S>,
        address: u64,
        bytes: u64,
    ) -> Result<(), PrivateError<B::Error>> {
        self.decommit_with(space, address, bytes, |_, _| {})
    }

    /// Runs an infallible, allocation-free invalidation after admission and
    /// metadata preparation, before any backing in this range can retire.
    pub fn decommit_with<
        F: FrameAllocator,
        B: VmBackend,
        const M: usize,
        const T: usize,
        S: SharedPage,
    >(
        &mut self,
        space: &mut AddressSpace<'_, F, B, M, T, S>,
        address: u64,
        bytes: u64,
        before: impl FnOnce(u64, u64),
    ) -> Result<(), PrivateError<B::Error>> {
        self.check_space(space)?;
        space.check_inactive().map_err(PrivateError::Vm)?;
        let (index, end, _) = self
            .range(address, bytes)
            .ok_or(PrivateError::InvalidRange)?;
        space
            .ensure_unleased(address, bytes)
            .map_err(PrivateError::Vm)?;
        let region = &mut self.regions[index];
        if region.closing {
            return Err(PrivateError::Conflict);
        }
        // A contiguous cut introduces at most two new boundaries.
        region
            .commits
            .try_reserve(2)
            .map_err(|_| PrivateError::OutOfResources)?;
        split_at(&mut region.commits, address);
        split_at(&mut region.commits, end);
        for part in &mut region.commits {
            if address <= part.start && part.end <= end {
                part.state = State::Retiring;
            }
        }
        before(address, end);
        retire(region, space)?;
        if self.retirement_pending() {
            Err(PrivateError::RetirementPending)
        } else {
            Ok(())
        }
    }

    pub fn release<
        F: FrameAllocator,
        B: VmBackend,
        const M: usize,
        const T: usize,
        S: SharedPage,
    >(
        &mut self,
        space: &mut AddressSpace<'_, F, B, M, T, S>,
        base: u64,
    ) -> Result<(), PrivateError<B::Error>> {
        self.release_with(space, base, |_, _| {})
    }

    /// Invalidates the admitted reservation before its first backing retirement.
    /// The callback must complete without allocating or failing.
    pub fn release_with<
        F: FrameAllocator,
        B: VmBackend,
        const M: usize,
        const T: usize,
        S: SharedPage,
    >(
        &mut self,
        space: &mut AddressSpace<'_, F, B, M, T, S>,
        base: u64,
        before: impl FnOnce(u64, u64),
    ) -> Result<(), PrivateError<B::Error>> {
        self.check_space(space)?;
        space.check_inactive().map_err(PrivateError::Vm)?;
        let index = self
            .regions
            .binary_search_by_key(&base, |region| region.start)
            .map_err(|_| PrivateError::InvalidRange)?;
        let region = &mut self.regions[index];
        space
            .ensure_unleased(region.start, region.end - region.start)
            .map_err(PrivateError::Vm)?;
        region.closing = true;
        for part in &mut region.commits {
            part.state = State::Retiring;
        }
        before(region.start, region.end);
        retire(region, space)?;
        self.regions.remove(index);
        if self.retirement_pending() {
            Err(PrivateError::RetirementPending)
        } else {
            Ok(())
        }
    }

    /// Failed retirements keep their owners and charges until a later retry.
    pub fn retry_retirement<
        F: FrameAllocator,
        B: VmBackend,
        const M: usize,
        const T: usize,
        S: SharedPage,
    >(
        &mut self,
        space: &mut AddressSpace<'_, F, B, M, T, S>,
    ) -> Result<(), PrivateError<B::Error>> {
        self.check_space(space)?;
        let mut index = 0;
        let mut failed = false;
        while index < self.regions.len() {
            let region = &mut self.regions[index];
            failed |= retire(region, space).is_err();
            if region.closing && region.commits.is_empty() {
                self.regions.remove(index);
            } else {
                index += 1;
            }
        }
        if failed || self.retirement_pending() {
            Err(PrivateError::RetirementPending)
        } else {
            Ok(())
        }
    }

    pub fn retirement_pending(&self) -> bool {
        self.regions.iter().any(|region| {
            region.closing
                || region
                    .commits
                    .iter()
                    .any(|part| part.state == State::Retiring)
        })
    }

    /// Rejects legacy mappings and unmaps anywhere inside a reservation.
    pub fn overlaps(&self, address: u64, bytes: u64) -> bool {
        let Some(end) = address.checked_add(bytes) else {
            return true;
        };
        bytes != 0
            && self
                .regions
                .iter()
                .any(|region| address < region.end && region.start < end)
    }

    /// Identities are internal lifetime keys, never user-selectable handles.
    pub fn identity(&self, address: u64) -> Option<(u64, u64)> {
        let region = self
            .regions
            .iter()
            .find(|region| region.start <= address && address < region.end)?;
        if region.closing {
            return None;
        }
        let part = region.commits.iter().find(|part| {
            part.state == State::Live && part.start <= address && address < part.end
        })?;
        Some((region.id, part.id))
    }

    /// Closing the root retires every data alias before process charges disappear.
    pub fn clear_after_close<
        F: FrameAllocator,
        B: VmBackend,
        const M: usize,
        const T: usize,
        S: SharedPage,
    >(
        &mut self,
        space: &AddressSpace<'_, F, B, M, T, S>,
    ) -> Result<(), PrivateError<B::Error>> {
        if self.space.is_some_and(|owner| owner != space.identity())
            || !matches!(space.root_frame(), Err(VmError::Closed))
        {
            return Err(PrivateError::Conflict);
        }
        self.regions.clear();
        self.space = None;
        Ok(())
    }

    fn check_space<
        F: FrameAllocator,
        B: VmBackend,
        const M: usize,
        const T: usize,
        S: SharedPage,
    >(
        &self,
        space: &AddressSpace<'_, F, B, M, T, S>,
    ) -> Result<(), PrivateError<B::Error>> {
        space.root_frame().map_err(PrivateError::Vm)?;
        if self.space.is_some_and(|owner| owner != space.identity()) {
            return Err(PrivateError::Conflict);
        }
        Ok(())
    }

    fn range(&self, address: u64, bytes: u64) -> Option<(usize, u64, usize)> {
        let pages = page_count(bytes)?;
        if !address.is_multiple_of(FRAME_SIZE) {
            return None;
        }
        let end = address.checked_add(bytes)?;
        let index = self
            .regions
            .partition_point(|region| region.start <= address)
            .checked_sub(1)?;
        (end <= self.regions[index].end).then_some((index, end, pages))
    }
}

impl Default for PrivateVm {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PrivateVm {
    fn drop(&mut self) {
        // An unclosed address space deliberately retains its frame owners.
        // Keep the corresponding quota ownership on the same failure path.
        for region in self.regions.drain(..) {
            core::mem::forget(region);
        }
    }
}

fn retire<F: FrameAllocator, B: VmBackend, const M: usize, const T: usize, S: SharedPage>(
    region: &mut Region,
    space: &mut AddressSpace<'_, F, B, M, T, S>,
) -> Result<(), PrivateError<B::Error>> {
    let mut failed = false;
    let mut index = 0;
    while index < region.commits.len() {
        let part = region.commits[index];
        if part.state == State::Live {
            index += 1;
            continue;
        }
        let before = space.owned_pages_in_range(part.start, part.end);
        failed |= space.unmap_private_range(part.start, part.end).is_err();
        let after = space.owned_pages_in_range(part.start, part.end);
        refund(region, before - after);
        if after == 0 {
            region.commits.remove(index);
        } else {
            index += 1;
        }
    }
    if failed {
        Err(PrivateError::RetirementPending)
    } else {
        Ok(())
    }
}

fn merge_charge(resident: &mut Option<Charge>, charge: Charge) {
    if let Some(resident) = resident {
        if let Err(charge) = resident.merge(charge) {
            core::mem::forget(charge);
            panic!("private VM account invariant");
        }
    } else {
        *resident = Some(charge);
    }
}

fn refund(region: &mut Region, pages: usize) {
    if pages == 0 {
        return;
    }
    let charge = region
        .resident
        .as_mut()
        .expect("private VM charge invariant");
    charge.refund(pages).expect("private VM refund invariant");
    if charge.amount() == 0 {
        region.resident.take();
    }
}

fn split_at(parts: &mut Vec<Commit>, address: u64) {
    if let Some(index) = parts
        .iter()
        .position(|part| part.start < address && address < part.end)
    {
        let part = parts[index];
        parts[index].end = address;
        parts.insert(
            index + 1,
            Commit {
                start: address,
                ..part
            },
        );
    }
}

fn page_count(bytes: u64) -> Option<usize> {
    (bytes != 0 && bytes.is_multiple_of(FRAME_SIZE)).then_some(())?;
    usize::try_from(bytes / FRAME_SIZE).ok()
}

fn align_up(address: u64, alignment: u64) -> Option<u64> {
    Some(address.checked_add(alignment - 1)? & !(alignment - 1))
}

fn take_id(next: &mut Option<u64>) -> Option<u64> {
    let id = (*next)?;
    *next = id.checked_add(1);
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::take_id;

    #[test]
    fn exhausted_lifetime_ids_never_wrap_or_reappear() {
        let mut next = Some(u64::MAX - 1);
        assert_eq!(take_id(&mut next), Some(u64::MAX - 1));
        assert_eq!(take_id(&mut next), Some(u64::MAX));
        assert_eq!(take_id(&mut next), None);
        assert_eq!(take_id(&mut next), None);
    }
}
