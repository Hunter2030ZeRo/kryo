use core::ptr::NonNull;
use core::sync::atomic::{Ordering, compiler_fence};
use x86_64::registers::control::Cr3;
use x86_64::registers::model_specific::Msr;
use x86_64::structures::paging::{PageTable, PageTableFlags};
use x86_64::{PhysAddr, VirtAddr};

use super::boot::{check_cpu, physical_width};
use super::direct_map::{kernel_page, physical_page};
use super::{DirectMap, PagingError};

const PAGE_COUNT: usize = 512;
const REGION_SIZE: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionPermissions {
    pub writable: bool,
    pub executable: bool,
    pub user: bool,
    pub uncached: bool,
}

/// A shared supervisor branch for bounded kernel stacks and MMIO mappings.
pub struct KernelRegion {
    root: NonNull<PageTable>,
    leaf: NonNull<PageTable>,
    root_frame: u64,
    tables: [u64; 3],
    base: VirtAddr,
    root_index: usize,
    physical_bits: u8,
    attached: bool,
}

impl KernelRegion {
    /// # Safety
    /// Run on the BSP with IRQs disabled and exclusive page-table access.
    /// Retain the three exclusively owned RAM table frames until detach.
    /// Initialize direct-map alias policy first, then attach this branch before
    /// cloning user roots. All such roots must be retired before detach.
    pub unsafe fn new(
        direct_map: DirectMap,
        base: u64,
        table_frames: [u64; 3],
    ) -> Result<Self, PagingError> {
        check_cpu()?;
        let base = region_base(base)?;
        let root_frame = Cr3::read().0.start_address().as_u64();
        let bits = physical_width();
        for (index, frame) in table_frames.iter().copied().enumerate() {
            physical_page(frame, bits)?;
            if frame == root_frame || table_frames[..index].contains(&frame) {
                return Err(PagingError::AliasedFrame);
            }
        }
        let mut root = direct_map.table_address(root_frame)?;
        let root_index = usize::from(base.p4_index());
        if !unsafe { root.as_ref() }[root_index].is_unused() {
            return Err(PagingError::SlotOccupied);
        }
        let mut tables = [
            direct_map.table_address(table_frames[0])?,
            direct_map.table_address(table_frames[1])?,
            direct_map.table_address(table_frames[2])?,
        ];
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
        for table in &mut tables {
            unsafe { table.as_ptr().write(PageTable::new()) };
        }
        unsafe {
            tables[0].as_mut()[usize::from(base.p3_index())]
                .set_addr(PhysAddr::new(table_frames[1]), flags);
            tables[1].as_mut()[usize::from(base.p2_index())]
                .set_addr(PhysAddr::new(table_frames[2]), flags);
        }
        compiler_fence(Ordering::Release);
        unsafe { root.as_mut()[root_index].set_addr(PhysAddr::new(table_frames[0]), flags) };
        super::kernel_map::flush_global();
        Ok(Self {
            root,
            leaf: tables[2],
            root_frame,
            tables: table_frames,
            base,
            root_index,
            physical_bits: bits,
            attached: true,
        })
    }

    pub fn address(&self, slot: usize) -> Result<u64, PagingError> {
        region_address(self.base.as_u64(), slot)
    }

    /// # Safety
    /// The caller exclusively owns this RAM frame and retains it until unmap
    /// completes. The mapping must not conflict with existing Rust references.
    pub unsafe fn map_ram(&mut self, slot: usize, physical: u64) -> Result<u64, PagingError> {
        unsafe { self.map(slot, physical, false) }
    }

    /// # Safety
    /// The caller has authority over this MMIO page and retains that authority
    /// until unmap. Existing aliases must have compatible memory types.
    /// Do not create ordinary RAM references through this alias.
    pub unsafe fn map_mmio(&mut self, slot: usize, physical: u64) -> Result<u64, PagingError> {
        check_uncached_pat()?;
        unsafe { self.map(slot, physical, true) }
    }

    pub fn permissions(&self, slot: usize) -> Result<RegionPermissions, PagingError> {
        self.check_context()?;
        self.address(slot)?;
        let entry = &unsafe { self.leaf.as_ref() }[slot];
        if !entry.flags().contains(PageTableFlags::PRESENT) {
            return Err(PagingError::NotMapped);
        }
        Ok(decode_permissions(entry.flags()))
    }

    /// # Safety
    /// No CPU or device operation may still use this virtual alias. In
    /// particular, do not unmap a running thread's kernel stack or live MMIO.
    pub unsafe fn unmap(&mut self, slot: usize) -> Result<u64, PagingError> {
        self.check_context()?;
        self.address(slot)?;
        let entry = &mut unsafe { self.leaf.as_mut() }[slot];
        if !entry.flags().contains(PageTableFlags::PRESENT) {
            return Err(PagingError::NotMapped);
        }
        let physical = entry.addr().as_u64();
        entry.set_unused();
        super::kernel_map::flush_global();
        Ok(physical)
    }

    /// # Safety
    /// Every user root sharing this branch has been retired. No CPU uses the
    /// branch, and all aliases have been unmapped before table ownership ends.
    pub unsafe fn detach(&mut self) -> Result<(), PagingError> {
        self.check_context()?;
        if unsafe { self.leaf.as_ref() }
            .iter()
            .any(|entry| !entry.is_unused())
        {
            return Err(PagingError::SlotOccupied);
        }
        unsafe { self.root.as_mut()[self.root_index].set_unused() };
        super::kernel_map::flush_global();
        self.attached = false;
        Ok(())
    }

    unsafe fn map(
        &mut self,
        slot: usize,
        physical: u64,
        uncached: bool,
    ) -> Result<u64, PagingError> {
        self.check_context()?;
        let address = self.address(slot)?;
        let frame = physical_page(physical, self.physical_bits)?;
        if physical == self.root_frame || self.tables.contains(&physical) {
            return Err(PagingError::AliasedFrame);
        }
        let leaf = unsafe { self.leaf.as_mut() };
        if !leaf[slot].is_unused() {
            return Err(PagingError::AlreadyMapped);
        }
        if leaf
            .iter()
            .any(|entry| entry.flags().contains(PageTableFlags::PRESENT) && entry.addr() == frame)
        {
            return Err(PagingError::AliasedFrame);
        }
        leaf[slot].set_addr(frame, leaf_flags(uncached));
        super::kernel_map::flush_global();
        Ok(address)
    }

    fn check_context(&self) -> Result<(), PagingError> {
        check_cpu()?;
        if !self.attached || Cr3::read().0.start_address().as_u64() != self.root_frame {
            return Err(PagingError::ContextChanged);
        }
        let entry = &unsafe { self.root.as_ref() }[self.root_index];
        if !entry.flags().contains(PageTableFlags::PRESENT)
            || entry.addr().as_u64() != self.tables[0]
        {
            return Err(PagingError::ContextChanged);
        }
        Ok(())
    }
}

fn region_base(address: u64) -> Result<VirtAddr, PagingError> {
    let address = kernel_page(address)?;
    if !address.as_u64().is_multiple_of(REGION_SIZE) || usize::from(address.p4_index()) == 511 {
        return Err(PagingError::InvalidAddress);
    }
    region_address(address.as_u64(), PAGE_COUNT - 1)?;
    Ok(address)
}

fn region_address(base: u64, slot: usize) -> Result<u64, PagingError> {
    if slot >= PAGE_COUNT {
        return Err(PagingError::InvalidAddress);
    }
    let address = base
        .checked_add(slot as u64 * 4096)
        .ok_or(PagingError::AddressOverflow)?;
    kernel_page(address)?;
    Ok(address)
}

fn leaf_flags(uncached: bool) -> PageTableFlags {
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
    if uncached {
        flags |= PageTableFlags::NO_CACHE | PageTableFlags::WRITE_THROUGH;
    }
    flags
}

fn decode_permissions(flags: PageTableFlags) -> RegionPermissions {
    RegionPermissions {
        writable: flags.contains(PageTableFlags::WRITABLE),
        executable: !flags.contains(PageTableFlags::NO_EXECUTE),
        user: flags.contains(PageTableFlags::USER_ACCESSIBLE),
        uncached: flags.contains(PageTableFlags::NO_CACHE | PageTableFlags::WRITE_THROUGH),
    }
}

fn check_uncached_pat() -> Result<(), PagingError> {
    check_cpu()?;
    if core::arch::x86_64::__cpuid(1).edx & (1 << 16) != 0 {
        // PCD=1/PWT=1/PAT=0 selects entry 3, which must encode UC.
        let pat = unsafe { Msr::new(0x277).read() };
        if (pat >> 24) & 0xff != 0 {
            return Err(PagingError::UnsupportedPaging);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_bounds_leave_room_for_guards_and_reject_image_slot() {
        let base = 0xffff_ff00_0000_0000;
        assert!(region_base(base).is_ok());
        assert_eq!(region_address(base, 511), Ok(base + 511 * 4096));
        assert_eq!(region_address(base, 512), Err(PagingError::InvalidAddress));
        assert_eq!(region_base(base + 4096), Err(PagingError::InvalidAddress));
        assert_eq!(
            region_base(0xffff_ffff_8000_0000),
            Err(PagingError::InvalidAddress)
        );
    }

    #[test]
    fn ram_and_mmio_are_supervisor_writable_nx() {
        let ram = decode_permissions(leaf_flags(false));
        assert!(ram.writable && !ram.executable && !ram.user && !ram.uncached);
        let mmio = decode_permissions(leaf_flags(true));
        assert!(mmio.writable && !mmio.executable && !mmio.user && mmio.uncached);
        assert!(!leaf_flags(true).contains(PageTableFlags::GLOBAL));
    }
}
