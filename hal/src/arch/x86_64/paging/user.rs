use core::ptr::NonNull;
use core::sync::atomic::{Ordering, compiler_fence};
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::{PageTable, PageTableFlags};
use x86_64::{PhysAddr, VirtAddr};

use super::boot::{check_cpu, physical_width};
use super::direct_map::physical_page;
use super::{DirectMap, PagingError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PagePermissions {
    pub writable: bool,
    pub executable: bool,
}

impl PagePermissions {
    fn flags(self) -> Result<PageTableFlags, PagingError> {
        if self.writable && self.executable {
            return Err(PagingError::InvalidPermissions);
        }
        let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        if self.writable {
            flags |= PageTableFlags::WRITABLE;
        }
        if !self.executable {
            flags |= PageTableFlags::NO_EXECUTE;
        }
        Ok(flags)
    }
}

/// Private lower-half tables with inherited supervisor kernel mappings.
pub struct UserPageTable {
    root: NonNull<PageTable>,
    root_frame: u64,
    boot_root: u64,
    direct_map: DirectMap,
    physical_bits: u8,
    retired: bool,
}

struct Location {
    table: NonNull<PageTable>,
    index: usize,
    missing: usize,
    ancestors: [u64; 4],
}

impl UserPageTable {
    /// # Safety
    /// The caller exclusively owns root_frame and retains it and all supplied
    /// table frames until retirement. Run on the BSP with IRQs disabled and no
    /// concurrent table edits. Kernel mappings and their backing tables remain
    /// valid for this object's lifetime. Initialize HHDM protection first.
    pub unsafe fn new(direct_map: DirectMap, root_frame: u64) -> Result<Self, PagingError> {
        check_cpu()?;
        let boot_root = Cr3::read().0.start_address().as_u64();
        let physical_bits = physical_width();
        physical_page(root_frame, physical_bits)?;
        if root_frame == boot_root {
            return Err(PagingError::AliasedFrame);
        }
        let source = direct_map.table_address(boot_root)?;
        let mut root = direct_map.table_address(root_frame)?;
        unsafe { root.as_ptr().write(PageTable::new()) };
        for index in 256..512 {
            let mut entry = unsafe { source.as_ref() }[index].clone();
            entry.set_flags(entry.flags() & !PageTableFlags::USER_ACCESSIBLE);
            unsafe { root.as_mut()[index] = entry };
        }
        Ok(Self {
            root,
            root_frame,
            boot_root,
            direct_map,
            physical_bits,
            retired: false,
        })
    }

    pub const fn root_frame(&self) -> u64 {
        self.root_frame
    }

    pub fn check_inactive(&self) -> Result<(), PagingError> {
        check_cpu()?;
        if self.retired {
            return Err(PagingError::ContextChanged);
        }
        let active = Cr3::read().0.start_address().as_u64();
        if active == self.root_frame {
            return Err(PagingError::ActiveAddressSpace);
        }
        if active != self.boot_root {
            return Err(PagingError::ContextChanged);
        }
        Ok(())
    }

    pub fn tables_needed(&self, virtual_address: u64) -> Result<usize, PagingError> {
        self.check_inactive()?;
        let location = self.locate(user_page(virtual_address)?)?;
        if location.missing == 0 && !unsafe { location.table.as_ref() }[location.index].is_unused()
        {
            return Err(PagingError::AlreadyMapped);
        }
        Ok(location.missing)
    }

    /// # Safety
    /// The caller owns the data frame and all new table frames. Retain used
    /// tables and data until unmap/retirement, and enforce permissions across
    /// aliases, including making executable data read-only through the HHDM.
    /// New table frames are distinct unused RAM, with writable HHDM aliases.
    pub unsafe fn map(
        &mut self,
        virtual_address: u64,
        physical: u64,
        permissions: PagePermissions,
        new_tables: &[u64],
    ) -> Result<usize, PagingError> {
        self.check_inactive()?;
        let address = user_page(virtual_address)?;
        let frame = physical_page(physical, self.physical_bits)?;
        let flags = permissions.flags()?;
        let mut location = self.locate(address)?;
        let count = location.missing;
        if count == 0 && !unsafe { location.table.as_ref() }[location.index].is_unused() {
            return Err(PagingError::AlreadyMapped);
        }
        if location.ancestors[..4 - count].contains(&physical) {
            return Err(PagingError::AliasedFrame);
        }
        validate_new_tables(
            new_tables,
            count,
            physical,
            &location.ancestors[..4 - count],
            self.physical_bits,
        )?;
        let mut tables = [NonNull::dangling(); 3];
        for index in 0..count {
            tables[index] = self.direct_map.table_address(new_tables[index])?;
        }

        let indices = indices(address);
        let parents =
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;
        if count == 0 {
            unsafe { location.table.as_mut()[location.index].set_addr(frame, flags) };
        } else {
            for table in tables.iter().take(count) {
                unsafe { table.as_ptr().write(PageTable::new()) };
            }
            for index in 0..count - 1 {
                unsafe {
                    tables[index].as_mut()[indices[4 - count + index]]
                        .set_addr(PhysAddr::new(new_tables[index + 1]), parents);
                }
            }
            unsafe { tables[count - 1].as_mut()[indices[3]].set_addr(frame, flags) };
            compiler_fence(Ordering::Release);
            unsafe {
                location.table.as_mut()[location.index]
                    .set_addr(PhysAddr::new(new_tables[0]), parents);
            }
        }
        Ok(count)
    }

    /// # Safety
    /// No CPU may use this root or retain its non-global translations. All
    /// access through the removed mapping must have ended.
    pub unsafe fn unmap(&mut self, virtual_address: u64) -> Result<u64, PagingError> {
        self.check_inactive()?;
        let mut location = self.locate(user_page(virtual_address)?)?;
        if location.missing != 0 {
            return Err(PagingError::NotMapped);
        }
        let entry = &mut unsafe { location.table.as_mut() }[location.index];
        if !entry.flags().contains(PageTableFlags::PRESENT) {
            return Err(PagingError::NotMapped);
        }
        let frame = entry.addr().as_u64();
        entry.set_unused();
        Ok(frame)
    }

    /// # Safety
    /// No CPU uses this root and all its mapping accesses have ended. Retain
    /// root and table frames until this operation returns.
    pub unsafe fn retire(&mut self) -> Result<(), PagingError> {
        self.check_inactive()?;
        unsafe { self.root.as_ptr().write(PageTable::new()) };
        super::kernel_map::flush_global();
        self.retired = true;
        Ok(())
    }

    fn locate(&self, address: VirtAddr) -> Result<Location, PagingError> {
        let indices = indices(address);
        let mut table = self.root;
        let mut ancestors = [0; 4];
        ancestors[0] = self.root_frame;
        for level in 0..3 {
            let entry = &unsafe { table.as_ref() }[indices[level]];
            if entry.is_unused() {
                return Ok(Location {
                    table,
                    index: indices[level],
                    missing: 3 - level,
                    ancestors,
                });
            }
            let required = PageTableFlags::PRESENT
                | PageTableFlags::USER_ACCESSIBLE
                | PageTableFlags::WRITABLE;
            if !entry.flags().contains(required)
                || entry
                    .flags()
                    .intersects(PageTableFlags::HUGE_PAGE | PageTableFlags::NO_EXECUTE)
            {
                return Err(PagingError::ContextChanged);
            }
            let physical = entry.addr().as_u64();
            if ancestors[..=level].contains(&physical) {
                return Err(PagingError::AliasedFrame);
            }
            ancestors[level + 1] = physical;
            table = self.direct_map.table_address(physical)?;
        }
        Ok(Location {
            table,
            index: indices[3],
            missing: 0,
            ancestors,
        })
    }
}

pub fn user_page(address: u64) -> Result<VirtAddr, PagingError> {
    if !address.is_multiple_of(4096) {
        return Err(PagingError::UnalignedAddress);
    }
    if !(4096..0x0000_8000_0000_0000).contains(&address) {
        return Err(PagingError::InvalidAddress);
    }
    VirtAddr::try_new(address).map_err(|_| PagingError::InvalidAddress)
}

fn indices(address: VirtAddr) -> [usize; 4] {
    [
        usize::from(address.p4_index()),
        usize::from(address.p3_index()),
        usize::from(address.p2_index()),
        usize::from(address.p1_index()),
    ]
}

pub(super) fn validate_new_tables(
    tables: &[u64],
    count: usize,
    data: u64,
    ancestors: &[u64],
    bits: u8,
) -> Result<(), PagingError> {
    if tables.len() < count {
        return Err(PagingError::InsufficientTables);
    }
    for (index, frame) in tables.iter().copied().take(count).enumerate() {
        physical_page(frame, bits)?;
        if frame == data || ancestors.contains(&frame) || tables[..index].contains(&frame) {
            return Err(PagingError::AliasedFrame);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_range_excludes_null_kernel_and_noncanonical_addresses() {
        for address in [0, 0x8000_0000_0000, 0xffff_8000_0000_0000] {
            assert_eq!(user_page(address), Err(PagingError::InvalidAddress));
        }
        assert!(user_page(0x7fff_ffff_f000).is_ok());
        assert_eq!(user_page(0x1001), Err(PagingError::UnalignedAddress));
    }

    #[test]
    fn permissions_never_allow_write_and_execute() {
        assert_eq!(
            PagePermissions {
                writable: true,
                executable: true
            }
            .flags(),
            Err(PagingError::InvalidPermissions)
        );
        let code = PagePermissions {
            writable: false,
            executable: true,
        }
        .flags()
        .unwrap();
        assert!(code.contains(PageTableFlags::USER_ACCESSIBLE));
        assert!(!code.intersects(
            PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE | PageTableFlags::GLOBAL
        ));
        let data = PagePermissions {
            writable: true,
            executable: false,
        }
        .flags()
        .unwrap();
        assert!(data.contains(PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE));
    }

    #[test]
    fn missing_table_frames_and_aliases_are_rejected_before_mutation() {
        assert_eq!(
            validate_new_tables(&[0x2000], 2, 0x5000, &[0x1000], 36),
            Err(PagingError::InsufficientTables)
        );
        for tables in [[0x2000, 0x2000], [0x2000, 0x5000], [0x1000, 0x3000]] {
            assert_eq!(
                validate_new_tables(&tables, 2, 0x5000, &[0x1000], 36),
                Err(PagingError::AliasedFrame)
            );
        }
        assert!(validate_new_tables(&[0x2000, 0x3000], 2, 0x5000, &[0x1000], 36).is_ok());
    }
}
