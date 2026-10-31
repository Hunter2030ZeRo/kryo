use super::{FrameAllocator, Permissions, VmBackend, VmError};
use crate::mm::frame::{FRAME_SIZE, OwnedFrame};

struct Mapping<'pool> {
    address: u64,
    frame: OwnedFrame<'pool>,
    permissions: Permissions,
    published: bool,
}

pub struct AddressSpace<
    'pool,
    F: FrameAllocator,
    B: VmBackend,
    const MAPPINGS: usize,
    const TABLES: usize,
> {
    allocator: &'pool F,
    backend: B,
    root: Option<OwnedFrame<'pool>>,
    mappings: [Option<Mapping<'pool>>; MAPPINGS],
    tables: [Option<OwnedFrame<'pool>>; TABLES],
    table_count: usize,
}

impl<'pool, F: FrameAllocator, B: VmBackend, const MAPPINGS: usize, const TABLES: usize>
    AddressSpace<'pool, F, B, MAPPINGS, TABLES>
{
    pub fn new(
        allocator: &'pool F,
        create: impl FnOnce(u64) -> Result<B, B::Error>,
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
            mappings: [const { None }; MAPPINGS],
            tables: [const { None }; TABLES],
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
            .iter()
            .position(Option::is_none)
            .ok_or(VmError::MappingCapacity)?;
        let needed = self
            .backend
            .tables_needed(address)
            .map_err(VmError::Backend)?;
        assert!(needed <= 3, "VM backend table count invariant");
        if needed > TABLES - self.table_count {
            return Err(VmError::TableCapacity);
        }
        let frame = self.allocator.allocate().map_err(VmError::Allocation)?;
        let mut prepared = [const { None }; 3];
        for index in 0..needed {
            match self.allocator.allocate() {
                Ok(table) => prepared[index] = Some(table),
                Err(error) => {
                    // No hardware operation has seen these frames.
                    for table in prepared.into_iter().flatten() {
                        unsafe { release(self.allocator, table) };
                    }
                    unsafe { release(self.allocator, frame) };
                    return Err(VmError::Allocation(error));
                }
            }
        }
        let mut physical = [0; 3];
        for index in 0..needed {
            physical[index] = prepared[index].as_ref().unwrap().address();
        }
        self.mappings[slot] = Some(Mapping {
            address,
            frame,
            permissions,
            published: false,
        });
        let data = self.mappings[slot].as_ref().unwrap().frame.address();
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
                for (index, table) in prepared.into_iter().enumerate() {
                    let Some(table) = table else { continue };
                    if index < used {
                        self.tables[self.table_count] = Some(table);
                        self.table_count += 1;
                    } else {
                        // The backend did not publish unused prepared tables.
                        unsafe { release(self.allocator, table) };
                    }
                }
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
                    unsafe { release(self.allocator, mapping.frame) };
                }
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
        // Inactivity excludes user writes; the mapping retains this initialized frame.
        unsafe {
            self.backend
                .read_frame(mapping.frame.address(), offset, output)
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
        for slot in 0..MAPPINGS {
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

    fn remove_mapping(&mut self, slot: usize) -> Result<(), VmError<B::Error>> {
        let mapping = self.mappings[slot].as_mut().unwrap();
        let physical = mapping.frame.address();
        if mapping.published {
            // unmap completes the TLB work before the alias can become writable.
            let removed =
                unsafe { self.backend.unmap(mapping.address) }.map_err(VmError::Backend)?;
            assert_eq!(removed, physical, "VM backend unmapped frame invariant");
            mapping.published = false;
        }
        unsafe { self.backend.restore_frame(physical) }.map_err(VmError::Backend)?;
        let mapping = self.mappings[slot].take().unwrap();
        unsafe { release(self.allocator, mapping.frame) };
        Ok(())
    }
}

fn validate_address(address: u64) -> Option<()> {
    ((FRAME_SIZE..0x0000_8000_0000_0000).contains(&address) && address.is_multiple_of(FRAME_SIZE))
        .then_some(())
}

unsafe fn release<F: FrameAllocator>(allocator: &F, frame: OwnedFrame<'_>) {
    if unsafe { allocator.release(frame) }.is_err() {
        panic!("VM frame ownership invariant");
    }
}
