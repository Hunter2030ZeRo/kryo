use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

static NEXT_SPACE_ID: AtomicU64 = AtomicU64::new(1);
const USER_END: u64 = 0x0000_8000_0000_0000;

use super::{FrameAllocator, Permissions, VmBackend, VmError};
use crate::mm::frame::{FRAME_SIZE, OwnedFrame};
use crate::mm::shared::{NoSharedPage, SharedPage};
use crate::storage::Slots;

enum Backing<'pool, S: SharedPage> {
    Private(OwnedFrame<'pool>),
    Shared(S),
}

impl<S: SharedPage> Backing<'_, S> {
    fn physical(&self) -> u64 {
        match self {
            Self::Private(frame) => frame.address(),
            Self::Shared(page) => page.physical(),
        }
    }
}

struct Mapping<'pool, S: SharedPage> {
    id: u64,
    address: u64,
    backing: Backing<'pool, S>,
    permissions: Permissions,
    published: bool,
}

/// Identifies one word in a particular private mapping lifetime, without
/// retaining its backing or granting authority over the address space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivateWordKey {
    space: u64,
    mapping: u64,
    address: u64,
}

impl PrivateWordKey {
    pub const fn space(self) -> u64 {
        self.space
    }

    pub const fn address(self) -> u64 {
        self.address
    }
}

/// Keeps declared execution ranges mapped until explicitly released by their
/// owner. Dropping the token deliberately leaves the protection in place.
#[derive(Debug)]
#[must_use = "Retain the execution lease until the thread has stopped"]
pub struct ExecutionLease {
    space: u64,
    id: u64,
}

#[derive(Clone, Copy)]
struct Span {
    start: u64,
    end: u64,
}

impl Span {
    fn checked(start: u64, bytes: u64) -> Option<Self> {
        let end = start.checked_add(bytes)?;
        (bytes != 0 && end <= USER_END).then_some(Self { start, end })
    }

    fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

struct ExecutionRange {
    id: u64,
    stack: Span,
    tls: Span,
    guard: Option<Span>,
}

impl ExecutionRange {
    fn spans(&self) -> [Option<Span>; 3] {
        [Some(self.stack), Some(self.tls), self.guard]
    }

    fn overlaps(&self, span: Span) -> bool {
        self.spans()
            .into_iter()
            .flatten()
            .any(|range| range.overlaps(span))
    }

    fn guard_obstacle(&self, start: u64, end: u64) -> Option<u64> {
        let guard = self.guard?;
        (start < guard.end && guard.start < end).then_some((guard.end - 1) & !(FRAME_SIZE - 1))
    }
}

pub struct AddressSpace<
    'pool,
    F: FrameAllocator,
    B: VmBackend,
    const MAPPINGS: usize,
    const TABLES: usize,
    S: SharedPage = NoSharedPage,
> {
    allocator: &'pool F,
    backend: B,
    identity: u64,
    root: Option<OwnedFrame<'pool>>,
    mappings: Slots<Option<Mapping<'pool, S>>, MAPPINGS>,
    mapping_count: usize,
    next_mapping: Option<u64>,
    executions: Vec<ExecutionRange>,
    next_execution: Option<u64>,
    tables: Slots<Option<OwnedFrame<'pool>>, TABLES>,
    table_count: usize,
}

impl<
    'pool,
    F: FrameAllocator,
    B: VmBackend,
    const MAPPINGS: usize,
    const TABLES: usize,
    S: SharedPage,
> AddressSpace<'pool, F, B, MAPPINGS, TABLES, S>
{
    pub fn new(
        allocator: &'pool F,
        create: impl FnOnce(u64) -> Result<B, B::Error>,
    ) -> Result<Self, VmError<B::Error>> {
        Self::create(allocator, create, None)
    }

    pub fn new_growing(
        allocator: &'pool F,
        create: impl FnOnce(u64) -> Result<B, B::Error>,
    ) -> Result<Self, VmError<B::Error>> {
        Self::create(allocator, create, Some((usize::MAX, usize::MAX)))
    }

    /// Limits mapping records and all owned user table frames, including root.
    /// Shared backing is charged by its memory object, independently of aliases.
    pub fn new_limited(
        allocator: &'pool F,
        create: impl FnOnce(u64) -> Result<B, B::Error>,
        mapping_limit: usize,
        table_limit: usize,
    ) -> Result<Self, VmError<B::Error>> {
        let intermediate = table_limit.checked_sub(1).ok_or(VmError::TableCapacity)?;
        Self::create(allocator, create, Some((mapping_limit, intermediate)))
    }

    fn create(
        allocator: &'pool F,
        create: impl FnOnce(u64) -> Result<B, B::Error>,
        limits: Option<(usize, usize)>,
    ) -> Result<Self, VmError<B::Error>> {
        let identity = NEXT_SPACE_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| VmError::MappingCapacity)?;
        let root = allocator.allocate().map_err(VmError::Allocation)?;
        let backend = match create(root.address()) {
            Ok(backend) => backend,
            Err(error) => {
                // A failed backend constructor must leave the root unpublished.
                unsafe { release(allocator, root) };
                return Err(VmError::Backend(error));
            }
        };
        assert_eq!(
            backend.root_frame(),
            root.address(),
            "VM backend root invariant"
        );
        // If the factory violates inactivity, retain ownership rather than free live storage.
        if let Err(error) = backend.check_inactive() {
            drop(root);
            return Err(VmError::Backend(error));
        }
        Ok(Self {
            allocator,
            backend,
            identity,
            root: Some(root),
            mappings: if let Some((mappings, _)) = limits {
                Slots::limited(mappings)
            } else {
                Slots::new([const { None }; MAPPINGS])
            },
            mapping_count: 0,
            next_mapping: Some(1),
            executions: Vec::new(),
            next_execution: Some(1),
            tables: if let Some((_, tables)) = limits {
                Slots::limited(tables)
            } else {
                Slots::new([const { None }; TABLES])
            },
            table_count: 0,
        })
    }

    /// A descriptive lifetime ID; it grants no address-space authority.
    pub fn identity(&self) -> u64 {
        self.identity
    }

    pub fn root_frame(&self) -> Result<u64, VmError<B::Error>> {
        Ok(self.root.as_ref().ok_or(VmError::Closed)?.address())
    }

    /// Protects private stack/TLS storage and an optional unmapped lower guard.
    /// Only range metadata is allocated; backing remains owned by this space.
    pub fn lease_execution(
        &mut self,
        stack_base: u64,
        stack_bytes: u64,
        tls_base: u64,
        tls_bytes: u64,
        guard_bytes: u64,
    ) -> Result<ExecutionLease, VmError<B::Error>> {
        self.check_inactive()?;
        let stack = Span::checked(stack_base, stack_bytes).ok_or(VmError::InvalidAddress)?;
        let tls = Span::checked(tls_base, tls_bytes).ok_or(VmError::InvalidAddress)?;
        if stack.start < FRAME_SIZE
            || tls.start < FRAME_SIZE
            || stack_bytes < 16
            || !stack.start.is_multiple_of(16)
            || !stack.end.is_multiple_of(16)
            || stack.overlaps(tls)
        {
            return Err(VmError::InvalidAddress);
        }
        let guard = if guard_bytes == 0 {
            None
        } else {
            if !guard_bytes.is_multiple_of(FRAME_SIZE) || !stack.start.is_multiple_of(FRAME_SIZE) {
                return Err(VmError::InvalidAddress);
            }
            let start = stack
                .start
                .checked_sub(guard_bytes)
                .ok_or(VmError::InvalidAddress)?;
            let guard = Span::checked(start, guard_bytes).ok_or(VmError::InvalidAddress)?;
            if guard.overlaps(tls) {
                return Err(VmError::InvalidAddress);
            }
            Some(guard)
        };
        let mut range = ExecutionRange {
            id: 0,
            stack,
            tls,
            guard,
        };
        if range
            .spans()
            .into_iter()
            .flatten()
            .any(|span| self.executions.iter().any(|owner| owner.overlaps(span)))
        {
            return Err(VmError::InUse);
        }
        self.check_execution_data(stack)?;
        self.check_execution_data(tls)?;
        if let Some(guard) = guard
            && self.last_occupied(guard.start, guard.end).is_some()
        {
            return Err(VmError::AlreadyMapped);
        }
        self.executions
            .try_reserve(1)
            .map_err(|_| VmError::MappingCapacity)?;
        range.id = take_execution_id(&mut self.next_execution).ok_or(VmError::MappingCapacity)?;
        let lease = ExecutionLease {
            space: self.identity,
            id: range.id,
        };
        self.executions.push(range);
        Ok(lease)
    }

    /// The scheduler must end execution before releasing this token. This only
    /// removes protection; the runtime still owns and may reuse the mappings.
    pub fn release_execution(&mut self, lease: &ExecutionLease) -> Result<(), VmError<B::Error>> {
        let index = self.execution_index(lease)?;
        self.executions.remove(index);
        Ok(())
    }

    /// A kernel-controlled TLS base must stay within the retained TLS storage.
    pub fn execution_tls_contains(
        &self,
        lease: &ExecutionLease,
        address: u64,
    ) -> Result<bool, VmError<B::Error>> {
        let range = self.executions[self.execution_index(lease)?].tls;
        Ok(range.start <= address && address < range.end)
    }

    fn execution_index(&self, lease: &ExecutionLease) -> Result<usize, VmError<B::Error>> {
        self.check_inactive()?;
        if lease.space != self.identity {
            return Err(VmError::InvalidAddress);
        }
        self.executions
            .binary_search_by_key(&lease.id, |range| range.id)
            .map_err(|_| VmError::InvalidAddress)
    }

    /// Reject retirement or replacement before any invalidation is published.
    pub fn ensure_unleased(&self, start: u64, bytes: u64) -> Result<(), VmError<B::Error>> {
        self.check_inactive()?;
        let span = Span::checked(start, bytes).ok_or(VmError::InvalidAddress)?;
        if self.executions.iter().any(|range| range.overlaps(span)) {
            Err(VmError::InUse)
        } else {
            Ok(())
        }
    }

    fn check_execution_data(&self, span: Span) -> Result<(), VmError<B::Error>> {
        let mut page = span.start & !(FRAME_SIZE - 1);
        let mut index = self.lower_bound(page);
        while page < span.end {
            let mapping = self.mappings[..self.mapping_count]
                .get(index)
                .and_then(Option::as_ref)
                .ok_or(VmError::NotMapped)?;
            if mapping.address != page || !mapping.published {
                return Err(VmError::NotMapped);
            }
            if mapping.permissions != Permissions::ReadWrite
                || !matches!(mapping.backing, Backing::Private(_))
            {
                return Err(VmError::InvalidPermissions);
            }
            page += FRAME_SIZE;
            index += 1;
        }
        Ok(())
    }

    pub fn map_page(
        &mut self,
        address: u64,
        permissions: Permissions,
        contents: &[u8],
    ) -> Result<(), VmError<B::Error>> {
        self.check_inactive()?;
        validate_address(address).ok_or(VmError::InvalidAddress)?;
        self.ensure_unleased(address, FRAME_SIZE)?;
        if contents.len() > FRAME_SIZE as usize {
            return Err(VmError::InvalidContents);
        }
        if self.find(address).is_some() {
            return Err(VmError::AlreadyMapped);
        }
        self.reserve_mappings(1)?;
        let id = self.reserve_mapping_ids(1)?;
        let frame = self.allocator.allocate().map_err(VmError::Allocation)?;
        self.map_owned_frame(id, address, permissions, contents, frame)
    }

    /// Obtains every data frame before publishing the first anonymous mapping.
    /// On failure, callers must retire any owners retained in this range.
    pub(crate) fn map_private_range(
        &mut self,
        address: u64,
        pages: usize,
    ) -> Result<(), VmError<B::Error>> {
        self.check_inactive()?;
        let end = range_end(address, pages).ok_or(VmError::InvalidAddress)?;
        self.ensure_unleased(address, end - address)?;
        if self.last_occupied(address, end).is_some() {
            return Err(VmError::AlreadyMapped);
        }
        self.reserve_mappings(pages)?;
        // Reserve the possible table metadata up to this space's configured cap.
        // Exact physical table requirements depend on previously published paths.
        if let Slots::Growing { values, limit } = &mut self.tables {
            let additional = pages
                .saturating_mul(3)
                .min(limit.saturating_sub(values.len()));
            values
                .try_reserve(additional)
                .map_err(|_| VmError::TableCapacity)?;
        }
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(pages)
            .map_err(|_| VmError::MappingCapacity)?;
        let first_id = self.reserve_mapping_ids(pages)?;
        for _ in 0..pages {
            match self.allocator.allocate() {
                Ok(frame) => frames.push(frame),
                Err(error) => {
                    for frame in frames {
                        unsafe { release(self.allocator, frame) };
                    }
                    return Err(VmError::Allocation(error));
                }
            }
        }
        let mut frames = frames.into_iter();
        for page in 0..pages {
            let frame = frames.next().unwrap();
            if let Err(error) = self.map_owned_frame(
                first_id + page as u64,
                address + page as u64 * FRAME_SIZE,
                Permissions::ReadWrite,
                &[],
                frame,
            ) {
                for frame in frames {
                    unsafe { release(self.allocator, frame) };
                }
                return Err(error);
            }
        }
        Ok(())
    }

    fn map_owned_frame(
        &mut self,
        id: u64,
        address: u64,
        permissions: Permissions,
        contents: &[u8],
        frame: OwnedFrame<'pool>,
    ) -> Result<(), VmError<B::Error>> {
        let prepared = (|| {
            let needed = self
                .backend
                .tables_needed(address)
                .map_err(VmError::Backend)?;
            assert!(needed <= 3, "VM backend table count invariant");
            let length = self
                .table_count
                .checked_add(needed)
                .ok_or(VmError::TableCapacity)?;
            self.tables
                .reserve_len(length, || None)
                .map_err(|_| VmError::TableCapacity)?;
            Ok((needed, self.prepare_tables(needed)?))
        })();
        let (needed, prepared) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                unsafe { release(self.allocator, frame) };
                return Err(error);
            }
        };
        let physical = table_addresses(&prepared);
        let slot = self.insert_mapping(Mapping {
            id,
            address,
            backing: Backing::Private(frame),
            permissions,
            published: false,
        });
        let data = self.mappings[slot].as_ref().unwrap().backing.physical();
        let result = unsafe {
            self.backend
                .initialize_frame(data, contents)
                .and_then(|()| self.backend.seal_frame(data, permissions))
                .and_then(|()| {
                    self.backend
                        .map(address, data, permissions, &physical[..needed])
                })
        };
        match result {
            Ok(used) => {
                assert!(used <= needed, "VM backend retained table count invariant");
                self.retain_tables(prepared, used);
                self.mappings[slot].as_mut().unwrap().published = true;
                Ok(())
            }
            Err(error) => {
                for table in prepared.into_iter().flatten() {
                    unsafe { release(self.allocator, table) };
                }
                // Unpublished frames still need their kernel alias restored.
                if unsafe { self.backend.restore_frame(data) }.is_ok() {
                    let mapping = self.mappings[slot].take().unwrap();
                    let Backing::Private(frame) = mapping.backing else {
                        unreachable!()
                    };
                    unsafe { release(self.allocator, frame) };
                    self.compact_mappings();
                }
                Err(VmError::Backend(error))
            }
        }
    }

    /// Retains the page's strong object lease until unmap completes its TLB work.
    /// Shared backing is initialized once by its owner and can never be executable.
    pub fn map_shared_page(
        &mut self,
        address: u64,
        permissions: Permissions,
        page: S,
    ) -> Result<(), VmError<B::Error>> {
        self.check_inactive()?;
        validate_address(address).ok_or(VmError::InvalidAddress)?;
        self.ensure_unleased(address, FRAME_SIZE)?;
        if permissions.executable() {
            return Err(VmError::InvalidPermissions);
        }
        if self.find(address).is_some() {
            return Err(VmError::AlreadyMapped);
        }
        self.reserve_mappings(1)?;
        let id = self.reserve_mapping_ids(1)?;
        let needed = self
            .backend
            .tables_needed(address)
            .map_err(VmError::Backend)?;
        assert!(needed <= 3, "VM backend table count invariant");
        let length = self
            .table_count
            .checked_add(needed)
            .ok_or(VmError::TableCapacity)?;
        self.tables
            .reserve_len(length, || None)
            .map_err(|_| VmError::TableCapacity)?;
        let prepared = self.prepare_tables(needed)?;
        let physical = table_addresses(&prepared);
        let slot = self.insert_mapping(Mapping {
            id,
            address,
            backing: Backing::Shared(page),
            permissions,
            published: false,
        });
        let data = self.mappings[slot].as_ref().unwrap().backing.physical();
        // Mapping shared RAM never changes its global alias or its contents.
        let result = unsafe {
            self.backend
                .map(address, data, permissions, &physical[..needed])
        };
        match result {
            Ok(used) => {
                assert!(used <= needed, "VM backend retained table count invariant");
                self.retain_tables(prepared, used);
                self.mappings[slot].as_mut().unwrap().published = true;
                Ok(())
            }
            Err(error) => {
                for table in prepared.into_iter().flatten() {
                    unsafe { release(self.allocator, table) };
                }
                self.mappings[slot].take();
                self.compact_mappings();
                Err(VmError::Backend(error))
            }
        }
    }

    pub fn unmap_page(&mut self, address: u64) -> Result<(), VmError<B::Error>> {
        self.unmap_page_with(address, || {})
    }

    /// Invalidate external users only after locating an owned mapping. Retained
    /// unpublished owners remain eligible for cleanup retries through this path.
    pub fn unmap_page_with(
        &mut self,
        address: u64,
        before: impl FnOnce(),
    ) -> Result<(), VmError<B::Error>> {
        self.check_inactive()?;
        self.ensure_unleased(address, FRAME_SIZE)?;
        let slot = self.find(address).ok_or(VmError::NotMapped)?;
        before();
        self.remove_mapping(slot)?;
        self.compact_mappings();
        Ok(())
    }

    /// Includes unpublished owners whose aliases are still awaiting retirement.
    pub(crate) fn owned_pages_in_range(&self, start: u64, end: u64) -> usize {
        let first = self.lower_bound(start);
        let last = self.lower_bound(end);
        last - first
    }

    /// Returns a page to advance past an overlapping obstacle. A guard may end
    /// beyond the query, so reservation search can skip its entire interval.
    pub(crate) fn last_occupied(&self, start: u64, end: u64) -> Option<u64> {
        let last = self.lower_bound(end);
        let mapped = last
            .checked_sub(1)
            .map(|index| self.mappings[index].as_ref().unwrap().address)
            .filter(|address| *address >= start);
        let guards = self
            .executions
            .iter()
            .filter_map(|range| range.guard_obstacle(start, end))
            .max();
        mapped.into_iter().chain(guards).max()
    }

    /// Retires a range in one metadata pass, even if individual retirements fail.
    pub(crate) fn unmap_private_range(
        &mut self,
        start: u64,
        end: u64,
    ) -> Result<(), VmError<B::Error>> {
        self.check_inactive()?;
        self.ensure_unleased(
            start,
            end.checked_sub(start).ok_or(VmError::InvalidAddress)?,
        )?;
        let first = self.lower_bound(start);
        let last = self.lower_bound(end);
        let mut failure = None;
        for slot in first..last {
            if !matches!(
                self.mappings[slot].as_ref().unwrap().backing,
                Backing::Private(_)
            ) {
                failure.get_or_insert(VmError::InvalidPermissions);
                continue;
            }
            if let Err(error) = self.remove_mapping(slot)
                && failure.is_none()
            {
                failure = Some(error);
            }
        }
        self.compact_mappings();
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Snapshot a private native word while the backend excludes user writers.
    /// No user reference or backing lease escapes this call.
    pub fn private_word(&self, address: u64) -> Result<(PrivateWordKey, u32), VmError<B::Error>> {
        let key = self.private_word_key(address)?;
        let mut bytes = [0; 4];
        self.read_page(
            address & !(FRAME_SIZE - 1),
            (address % FRAME_SIZE) as usize,
            &mut bytes,
        )?;
        Ok((key, u32::from_ne_bytes(bytes)))
    }

    /// Resolve a private RW word without reading its contents. Queues must
    /// invalidate this identity before the mapping can retire or be reused.
    pub fn private_word_key(&self, address: u64) -> Result<PrivateWordKey, VmError<B::Error>> {
        self.check_inactive()?;
        if !address.is_multiple_of(4) {
            return Err(VmError::InvalidAddress);
        }
        let base = address & !(FRAME_SIZE - 1);
        validate_address(base).ok_or(VmError::InvalidAddress)?;
        let slot = self.find(base).ok_or(VmError::NotMapped)?;
        let mapping = self.mappings[slot].as_ref().unwrap();
        if !mapping.published {
            return Err(VmError::NotMapped);
        }
        if mapping.permissions != Permissions::ReadWrite
            || !matches!(mapping.backing, Backing::Private(_))
        {
            return Err(VmError::InvalidPermissions);
        }
        Ok(PrivateWordKey {
            space: self.identity,
            mapping: mapping.id,
            address,
        })
    }

    pub fn read_page(
        &self,
        address: u64,
        offset: usize,
        output: &mut [u8],
    ) -> Result<(), VmError<B::Error>> {
        self.check_inactive()?;
        if offset
            .checked_add(output.len())
            .is_none_or(|end| end > FRAME_SIZE as usize)
        {
            return Err(VmError::InvalidContents);
        }
        let slot = self.find(address).ok_or(VmError::NotMapped)?;
        let mapping = self.mappings[slot].as_ref().unwrap();
        if !mapping.published {
            return Err(VmError::NotMapped);
        }
        // The backend excludes writers across all roots, including shared aliases.
        unsafe {
            self.backend
                .read_frame(mapping.backing.physical(), offset, output)
        }
        .map_err(VmError::Backend)
    }

    pub fn permissions(&self, address: u64) -> Option<Permissions> {
        self.find(address)
            .and_then(|slot| self.mappings[slot].as_ref())
            .filter(|mapping| mapping.published)
            .map(|mapping| mapping.permissions)
    }

    pub fn mapped_pages(&self) -> usize {
        self.mappings
            .iter()
            .flatten()
            .filter(|mapping| mapping.published)
            .count()
    }

    pub fn retained_tables(&self) -> usize {
        self.table_count
    }

    pub fn owned_tables(&self) -> usize {
        self.table_count + usize::from(self.root.is_some())
    }

    /// The exclusive borrow prevents mapping mutation or close during execution.
    /// The callback must obey the unsafe entry routine's CPU/root restoration contract.
    pub fn with_root<R>(&mut self, run: impl FnOnce(u64) -> R) -> Result<R, VmError<B::Error>> {
        self.check_inactive()?;
        let result = run(self.root_frame()?);
        self.check_inactive()?;
        Ok(result)
    }

    /// Close explicitly. Failure retains all frames that are not yet safe to release.
    /// Dropping an unclosed space leaks ownership rather than reclaiming active memory.
    pub fn close(&mut self) -> Result<(), VmError<B::Error>> {
        self.check_inactive()?;
        if !self.executions.is_empty() {
            return Err(VmError::InUse);
        }
        for slot in 0..self.mappings.len() {
            if self.mappings[slot].is_some()
                && let Err(error) = self.remove_mapping(slot)
            {
                self.compact_mappings();
                return Err(error);
            }
        }
        self.mapping_count = 0;
        // All data access is retired; the backend unlinks the inactive root and flushes.
        unsafe { self.backend.retire() }.map_err(VmError::Backend)?;
        for table in &mut self.tables[..self.table_count] {
            unsafe { release(self.allocator, table.take().unwrap()) };
        }
        self.table_count = 0;
        unsafe { release(self.allocator, self.root.take().unwrap()) };
        self.mapping_count = 0;
        self.mappings.release();
        self.tables.release();
        self.executions = Vec::new();
        Ok(())
    }

    pub(super) fn check_inactive(&self) -> Result<(), VmError<B::Error>> {
        self.root_frame()?;
        self.backend.check_inactive().map_err(VmError::Backend)
    }

    fn lower_bound(&self, address: u64) -> usize {
        self.mappings[..self.mapping_count]
            .partition_point(|entry| entry.as_ref().unwrap().address < address)
    }

    fn find(&self, address: u64) -> Option<usize> {
        let slot = self.lower_bound(address);
        (slot < self.mapping_count && self.mappings[slot].as_ref().unwrap().address == address)
            .then_some(slot)
    }

    fn reserve_mappings(&mut self, additional: usize) -> Result<(), VmError<B::Error>> {
        let length = self
            .mapping_count
            .checked_add(additional)
            .ok_or(VmError::MappingCapacity)?;
        self.mappings
            .reserve_len(length, || None)
            .map_err(|_| VmError::MappingCapacity)
    }

    fn reserve_mapping_ids(&mut self, count: usize) -> Result<u64, VmError<B::Error>> {
        take_mapping_ids(&mut self.next_mapping, count).ok_or(VmError::MappingCapacity)
    }

    fn insert_mapping(&mut self, mapping: Mapping<'pool, S>) -> usize {
        let slot = self.lower_bound(mapping.address);
        for index in (slot..self.mapping_count).rev() {
            self.mappings[index + 1] = self.mappings[index].take();
        }
        self.mappings[slot] = Some(mapping);
        self.mapping_count += 1;
        slot
    }

    fn compact_mappings(&mut self) {
        let mut target = 0;
        for source in 0..self.mapping_count {
            if self.mappings[source].is_some() {
                if source != target {
                    self.mappings[target] = self.mappings[source].take();
                }
                target += 1;
            }
        }
        self.mapping_count = target;
    }

    fn prepare_tables(
        &self,
        count: usize,
    ) -> Result<[Option<OwnedFrame<'pool>>; 3], VmError<B::Error>> {
        let mut tables = [const { None }; 3];
        for index in 0..count {
            match self.allocator.allocate() {
                Ok(table) => tables[index] = Some(table),
                Err(error) => {
                    // Hardware has not seen any of these frames.
                    for table in tables.into_iter().flatten() {
                        unsafe { release(self.allocator, table) };
                    }
                    return Err(VmError::Allocation(error));
                }
            }
        }
        Ok(tables)
    }

    fn retain_tables(&mut self, tables: [Option<OwnedFrame<'pool>>; 3], used: usize) {
        for (index, table) in tables.into_iter().enumerate() {
            let Some(table) = table else { continue };
            if index < used {
                self.tables[self.table_count] = Some(table);
                self.table_count += 1;
            } else {
                // A successful backend map did not publish this unused frame.
                unsafe { release(self.allocator, table) };
            }
        }
    }

    fn remove_mapping(&mut self, slot: usize) -> Result<(), VmError<B::Error>> {
        let mapping = self.mappings[slot].as_mut().unwrap();
        let physical = mapping.backing.physical();
        if mapping.published {
            // unmap completes the TLB work before the alias can become writable.
            let removed =
                unsafe { self.backend.unmap(mapping.address) }.map_err(VmError::Backend)?;
            assert_eq!(removed, physical, "VM backend unmapped frame invariant");
            mapping.published = false;
        }
        if matches!(mapping.backing, Backing::Private(_)) {
            unsafe { self.backend.restore_frame(physical) }.map_err(VmError::Backend)?;
        }
        let mapping = self.mappings[slot].take().unwrap();
        if let Backing::Private(frame) = mapping.backing {
            unsafe { release(self.allocator, frame) };
        }
        Ok(())
    }
}

impl<F: FrameAllocator, B: VmBackend, const MAPPINGS: usize, const TABLES: usize, S: SharedPage>
    Drop for AddressSpace<'_, F, B, MAPPINGS, TABLES, S>
{
    fn drop(&mut self) {
        for mapping in &mut self.mappings {
            if let Some(mapping) = mapping.take() {
                // Unclosed page tables may still reference this backing. A
                // forgotten strong lease must retain the entire shared object.
                core::mem::forget(mapping);
            }
        }
    }
}

fn validate_address(address: u64) -> Option<()> {
    ((FRAME_SIZE..0x0000_8000_0000_0000).contains(&address) && address.is_multiple_of(FRAME_SIZE))
        .then_some(())
}

fn table_addresses(tables: &[Option<OwnedFrame<'_>>; 3]) -> [u64; 3] {
    core::array::from_fn(|index| tables[index].as_ref().map_or(0, OwnedFrame::address))
}

unsafe fn release<F: FrameAllocator>(allocator: &F, frame: OwnedFrame<'_>) {
    if unsafe { allocator.release(frame) }.is_err() {
        panic!("VM frame ownership invariant");
    }
}

fn range_end(address: u64, pages: usize) -> Option<u64> {
    validate_address(address)?;
    if pages == 0 {
        return None;
    }
    let bytes = u64::try_from(pages).ok()?.checked_mul(FRAME_SIZE)?;
    let end = address.checked_add(bytes)?;
    (end <= 0x0000_8000_0000_0000).then_some(end)
}

fn take_mapping_ids(next: &mut Option<u64>, count: usize) -> Option<u64> {
    let first = (*next)?;
    let additional = u64::try_from(count.checked_sub(1)?).ok()?;
    let last = first.checked_add(additional)?;
    *next = last.checked_add(1);
    Some(first)
}

fn take_execution_id(next: &mut Option<u64>) -> Option<u64> {
    let id = (*next)?;
    *next = id.checked_add(1);
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::{ExecutionRange, Span, take_execution_id, take_mapping_ids};
    use crate::mm::frame::FRAME_SIZE;

    #[test]
    fn mapping_identity_exhaustion_never_partially_reserves_or_wraps() {
        let mut next = Some(u64::MAX - 1);
        assert_eq!(take_mapping_ids(&mut next, 3), None);
        assert_eq!(next, Some(u64::MAX - 1));
        assert_eq!(take_mapping_ids(&mut next, 0), None);
        assert_eq!(next, Some(u64::MAX - 1));
        assert_eq!(take_mapping_ids(&mut next, 2), Some(u64::MAX - 1));
        assert_eq!(next, None);
        assert_eq!(take_mapping_ids(&mut next, 1), None);
    }

    #[test]
    fn execution_identity_exhaustion_does_not_wrap_or_reuse() {
        let mut next = Some(u64::MAX);
        assert_eq!(take_execution_id(&mut next), Some(u64::MAX));
        assert_eq!(next, None);
        assert_eq!(take_execution_id(&mut next), None);
    }

    #[test]
    fn guard_navigation_skips_the_full_interval_for_a_single_page_query() {
        let start = 0x1_0000_0000;
        let end = 0x400_0000_0000;
        let range = ExecutionRange {
            id: 1,
            stack: Span {
                start: end,
                end: end + FRAME_SIZE,
            },
            tls: Span {
                start: end + FRAME_SIZE,
                end: end + 2 * FRAME_SIZE,
            },
            guard: Some(Span { start, end }),
        };
        assert_eq!(
            range.guard_obstacle(start, start + FRAME_SIZE),
            Some(end - FRAME_SIZE)
        );
        assert_eq!(range.guard_obstacle(start - FRAME_SIZE, start), None);
        assert_eq!(range.guard_obstacle(end, end + FRAME_SIZE), None);
    }
}
