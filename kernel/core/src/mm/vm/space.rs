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
    address: u64,
    backing: Backing<'pool, S>,
    permissions: Permissions,
    published: bool,
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
    root: Option<OwnedFrame<'pool>>,
    mappings: Slots<Option<Mapping<'pool, S>>, MAPPINGS>,
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
            root: Some(root),
            mappings: if let Some((mappings, _)) = limits {
                Slots::limited(mappings)
            } else {
                Slots::new([const { None }; MAPPINGS])
            },
            tables: if let Some((_, tables)) = limits {
                Slots::limited(tables)
            } else {
                Slots::new([const { None }; TABLES])
            },
            table_count: 0,
        })
    }

    pub fn root_frame(&self) -> Result<u64, VmError<B::Error>> {
        Ok(self.root.as_ref().ok_or(VmError::Closed)?.address())
    }

    pub fn map_page(
        &mut self,
        address: u64,
        permissions: Permissions,
        contents: &[u8],
    ) -> Result<(), VmError<B::Error>> {
        self.check_inactive()?;
        validate_address(address).ok_or(VmError::InvalidAddress)?;
        if contents.len() > FRAME_SIZE as usize {
            return Err(VmError::InvalidContents);
        }
        if self.find(address).is_some() {
            return Err(VmError::AlreadyMapped);
        }
        let slot = self
            .mappings
            .vacant(Option::is_none, || None)
            .map_err(|_| VmError::MappingCapacity)?;
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
        let frame = self.allocator.allocate().map_err(VmError::Allocation)?;
        let prepared = match self.prepare_tables(needed) {
            Ok(tables) => tables,
            Err(error) => {
                unsafe { release(self.allocator, frame) };
                return Err(error);
            }
        };
        let physical = table_addresses(&prepared);
        self.mappings[slot] = Some(Mapping {
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
                // Failed publication cannot retain the prepared user page tables.
                for table in prepared.into_iter().flatten() {
                    unsafe { release(self.allocator, table) };
                }
                // Keep the data owner for close retry if alias restoration fails.
                if unsafe { self.backend.restore_frame(data) }.is_ok() {
                    let mapping = self.mappings[slot].take().unwrap();
                    let Backing::Private(frame) = mapping.backing else {
                        unreachable!("Private mapping backing invariant");
                    };
                    unsafe { release(self.allocator, frame) };
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
        if permissions.executable() {
            return Err(VmError::InvalidPermissions);
        }
        if self.find(address).is_some() {
            return Err(VmError::AlreadyMapped);
        }
        let slot = self
            .mappings
            .vacant(Option::is_none, || None)
            .map_err(|_| VmError::MappingCapacity)?;
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
        self.mappings[slot] = Some(Mapping {
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
                Err(VmError::Backend(error))
            }
        }
    }

    pub fn unmap_page(&mut self, address: u64) -> Result<(), VmError<B::Error>> {
        self.check_inactive()?;
        let slot = self.find(address).ok_or(VmError::NotMapped)?;
        self.remove_mapping(slot)
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
        for slot in 0..self.mappings.len() {
            if self.mappings[slot].is_some() {
                self.remove_mapping(slot)?;
            }
        }
        // All data access is retired; the backend unlinks the inactive root and flushes.
        unsafe { self.backend.retire() }.map_err(VmError::Backend)?;
        for table in &mut self.tables[..self.table_count] {
            unsafe { release(self.allocator, table.take().unwrap()) };
        }
        self.table_count = 0;
        unsafe { release(self.allocator, self.root.take().unwrap()) };
        self.mappings.release();
        self.tables.release();
        Ok(())
    }

    fn check_inactive(&self) -> Result<(), VmError<B::Error>> {
        self.root_frame()?;
        self.backend.check_inactive().map_err(VmError::Backend)
    }

    fn find(&self, address: u64) -> Option<usize> {
        self.mappings.iter().position(|mapping| {
            mapping
                .as_ref()
                .is_some_and(|mapping| mapping.address == address)
        })
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
