use core::ptr::NonNull;
use core::sync::atomic::{Ordering, compiler_fence};
use x86_64::instructions::{interrupts, tlb};
use x86_64::registers::control::{Cr3, Cr4, Cr4Flags};
use x86_64::registers::model_specific::{Efer, EferFlags};
use x86_64::structures::paging::{PageTable, PageTableFlags};
use x86_64::{PhysAddr, VirtAddr};

use super::direct_map::{kernel_page, physical_page};
use super::{DirectMap, PagingError};

/// One supervisor page in a dedicated branch of the active boot address space.
/// This is a BSP bootstrap mechanism, not a general address-space manager.
pub struct BootPageSlot {
    root: NonNull<PageTable>,
    leaf: NonNull<PageTable>,
    root_frame: u64,
    tables: [u64; 3],
    address: VirtAddr,
    indices: [usize; 4],
    physical_bits: u8,
    mapped: Option<u64>,
    attached: bool,
}

impl BootPageSlot {
    /// # Safety
    /// Run on the BSP before APs or scheduling, with IRQs disabled. Stay on
    /// this CPU and CR3, and exclude other changes to this branch until detach
    /// or drop. No AP may start until this temporary slot is detached/dropped.
    /// The root and three distinct, exclusively owned table frames
    /// must remain accessible through direct_map for the entire slot lifetime.
    /// Table-frame ownership cannot be released until detach/drop has finished.
    pub unsafe fn new(
        direct_map: DirectMap,
        virtual_address: u64,
        table_frames: [u64; 3],
    ) -> Result<Self, PagingError> {
        check_cpu()?;
        let address = kernel_page(virtual_address)?;
        let indices = page_indices(address);
        let root_frame = Cr3::read().0.start_address().as_u64();
        let physical_bits = physical_width();
        validate_tables(table_frames, root_frame, physical_bits)?;
        let mut root = direct_map.table_address(root_frame)?;
        let mut tables = [
            direct_map.table_address(table_frames[0])?,
            direct_map.table_address(table_frames[1])?,
            direct_map.table_address(table_frames[2])?,
        ];
        if !unsafe { root.as_ref() }[indices[0]].is_unused() {
            return Err(PagingError::SlotOccupied);
        }

        // Prepare the complete branch before publishing any root entry.
        for table in &mut tables {
            unsafe { table.as_ptr().write(PageTable::new()) };
        }
        let parent_flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
        unsafe {
            tables[0].as_mut()[indices[1]].set_addr(PhysAddr::new(table_frames[1]), parent_flags);
            tables[1].as_mut()[indices[2]].set_addr(PhysAddr::new(table_frames[2]), parent_flags);
        }
        compiler_fence(Ordering::Release);
        unsafe {
            root.as_mut()[indices[0]].set_addr(PhysAddr::new(table_frames[0]), parent_flags);
        }
        tlb::flush_all();

        Ok(Self {
            root,
            leaf: tables[2],
            root_frame,
            tables: table_frames,
            address,
            indices,
            physical_bits,
            mapped: None,
            attached: true,
        })
    }

    pub const fn virtual_address(&self) -> u64 {
        self.address.as_u64()
    }

    /// # Safety
    /// The caller owns this RAM frame, keeps it alive until unmap/detach, and
    /// controls every access through this alias. No incompatible references or
    /// permissions may exist. Only this new leaf is NX; boot aliases are unchanged.
    /// References through this alias must end before unmap, detach or drop.
    pub unsafe fn map(&mut self, physical: u64) -> Result<(), PagingError> {
        self.check_context()?;
        if self.mapped.is_some() {
            return Err(PagingError::AlreadyMapped);
        }
        let frame = physical_page(physical, self.physical_bits)?;
        if physical == self.root_frame || self.tables.contains(&physical) {
            return Err(PagingError::AliasedFrame);
        }
        let entry = &mut unsafe { self.leaf.as_mut() }[self.indices[3]];
        if !entry.is_unused() {
            return Err(PagingError::ContextChanged);
        }
        entry.set_addr(
            frame,
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
        );
        tlb::flush(self.address);
        self.mapped = Some(physical);
        Ok(())
    }

    pub fn unmap(&mut self) -> Result<u64, PagingError> {
        self.check_context()?;
        let physical = self.mapped.ok_or(PagingError::NotMapped)?;
        unsafe { self.leaf.as_mut()[self.indices[3]].set_unused() };
        tlb::flush(self.address);
        self.mapped = None;
        Ok(physical)
    }

    /// Remove the entire owned branch and flush before table frames are reusable.
    pub fn detach(mut self) -> Result<(), PagingError> {
        self.check_context()?;
        self.clear_branch();
        Ok(())
    }

    fn check_context(&self) -> Result<(), PagingError> {
        check_cpu()?;
        if Cr3::read().0.start_address().as_u64() != self.root_frame {
            return Err(PagingError::ContextChanged);
        }
        let entry = &unsafe { self.root.as_ref() }[self.indices[0]];
        if !entry.flags().contains(PageTableFlags::PRESENT)
            || entry.addr().as_u64() != self.tables[0]
        {
            return Err(PagingError::ContextChanged);
        }
        Ok(())
    }

    fn clear_branch(&mut self) {
        unsafe { self.root.as_mut()[self.indices[0]].set_unused() };
        tlb::flush_all();
        self.mapped = None;
        self.attached = false;
    }
}

impl Drop for BootPageSlot {
    fn drop(&mut self) {
        if self.attached {
            assert!(self.check_context().is_ok(), "Boot paging context changed");
            self.clear_branch();
        }
    }
}

pub(super) fn check_cpu() -> Result<(), PagingError> {
    if interrupts::are_enabled() {
        return Err(PagingError::InterruptsEnabled);
    }
    if !super::super::smp::all_users_quiescent() {
        return Err(PagingError::UsersActive);
    }
    if super::super::cpu::current_cpu().is_none_or(|cpu| cpu.index() != 0) {
        return Err(PagingError::ContextChanged);
    }
    if Cr4::read().intersects(Cr4Flags::L5_PAGING | Cr4Flags::PCID)
        || !Efer::read().contains(EferFlags::NO_EXECUTE_ENABLE)
    {
        return Err(PagingError::UnsupportedPaging);
    }
    Ok(())
}

pub(super) fn physical_width() -> u8 {
    let maximum = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    if maximum >= 0x8000_0008 {
        (core::arch::x86_64::__cpuid(0x8000_0008).eax & 0xff) as u8
    } else {
        36
    }
}

fn page_indices(address: VirtAddr) -> [usize; 4] {
    [
        usize::from(address.p4_index()),
        usize::from(address.p3_index()),
        usize::from(address.p2_index()),
        usize::from(address.p1_index()),
    ]
}

fn validate_tables(tables: [u64; 3], root: u64, bits: u8) -> Result<(), PagingError> {
    physical_page(root, bits)?;
    for (index, frame) in tables.iter().copied().enumerate() {
        physical_page(frame, bits)?;
        if frame == root || tables[..index].contains(&frame) {
            return Err(PagingError::AliasedFrame);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_frames_must_be_distinct_from_each_other_and_root() {
        assert_eq!(
            validate_tables([0x2000, 0x3000, 0x4000], 0x1000, 36),
            Ok(())
        );
        assert_eq!(
            validate_tables([0x2000, 0x3000, 0x2000], 0x1000, 36),
            Err(PagingError::AliasedFrame)
        );
        assert_eq!(
            validate_tables([0x2000, 0x1000, 0x4000], 0x1000, 36),
            Err(PagingError::AliasedFrame)
        );
        assert_eq!(
            validate_tables([0x2000, 0x3001, 0x4000], 0x1000, 36),
            Err(PagingError::UnalignedAddress)
        );
    }

    #[test]
    fn scratch_address_indices_do_not_share_kernel_image_slot() {
        let address = kernel_page(0xffff_a000_0000_0000).unwrap();
        assert_eq!(page_indices(address), [320, 0, 0, 0]);
        assert_eq!(page_indices(VirtAddr::new(0xffff_ffff_8000_0000))[0], 511);
    }
}
