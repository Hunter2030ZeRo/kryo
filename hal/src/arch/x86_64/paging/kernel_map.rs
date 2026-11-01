use core::ptr::NonNull;
use core::sync::atomic::{Ordering, compiler_fence};
use x86_64::registers::control::{Cr0, Cr0Flags, Cr3};
use x86_64::structures::paging::{PageTable, PageTableFlags, page_table::PageTableEntry};
use x86_64::{PhysAddr, VirtAddr};

use super::boot::{check_cpu, physical_width};
use super::user::validate_new_tables;
use super::{DirectMap, KernelRegion, PagePermissions, PagingError};

const TWO_MIB: u64 = 1 << 21;
const ONE_GIB: u64 = 1 << 30;

pub struct KernelDirectMap {
    direct_map: DirectMap,
    root: NonNull<PageTable>,
    root_frame: u64,
    physical_end: u64,
    physical_bits: u8,
}

struct Alias {
    table: NonNull<PageTable>,
    index: usize,
    level: usize,
    entry: PageTableEntry,
    ancestors: [u64; 4],
    writable: bool,
    executable: bool,
}

impl KernelDirectMap {
    /// # Safety
    /// Run once on the BSP with IRQs disabled before cloning user roots.
    /// The trusted boot layout maps managed RAM only through the HHDM; kernel
    /// image mappings in slot 511 do not alias these Usable frames. Retain
    /// all kernel mappings and supplied split-table frames for kernel lifetime.
    pub unsafe fn new(direct_map: DirectMap, physical_end: u64) -> Result<Self, PagingError> {
        unsafe { Self::initialize(direct_map, physical_end, None) }
    }

    /// # Safety
    /// The requirements of new apply. The additional branch belongs to region
    /// and currently contains only authorized MMIO, with no Usable RAM aliases.
    /// Retain its mappings, MMIO authority and table frames for kernel lifetime;
    /// later RAM mappings must use owned frames under this map's W^X policy.
    pub unsafe fn with_mmio_region(
        direct_map: DirectMap,
        physical_end: u64,
        region: &KernelRegion,
    ) -> Result<Self, PagingError> {
        let index = region.mmio_root_index(direct_map)?;
        unsafe { Self::initialize(direct_map, physical_end, Some(index)) }
    }

    unsafe fn initialize(
        direct_map: DirectMap,
        physical_end: u64,
        mmio_index: Option<usize>,
    ) -> Result<Self, PagingError> {
        check_cpu()?;
        let physical_bits = physical_width();
        if physical_end == 0 {
            return Err(PagingError::InvalidAddress);
        }
        physical_page_bound(physical_end, physical_bits)?;
        let first = VirtAddr::new(direct_map.frame_address(0)?);
        let last = VirtAddr::new(direct_map.frame_address(physical_end - 4096)?);
        let start = usize::from(first.p4_index());
        let end = usize::from(last.p4_index());
        if start < 256 || end >= 511 {
            return Err(PagingError::UnsupportedPaging);
        }
        let root_frame = Cr3::read().0.start_address().as_u64();
        let mut root = direct_map.table_address(root_frame)?;
        validate_root(unsafe { root.as_ref() }, start, end, mmio_index)?;

        // Upper entries are copied into user roots later; set shared policy first.
        unsafe {
            Cr0::write(Cr0::read() | Cr0Flags::WRITE_PROTECT);
            for index in start..=end {
                let entry = &mut root.as_mut()[index];
                if !entry.is_unused() {
                    let flags = (entry.flags() | PageTableFlags::NO_EXECUTE)
                        & !PageTableFlags::USER_ACCESSIBLE;
                    entry.set_flags(flags);
                }
            }
        }
        flush_global()?;
        Ok(Self {
            direct_map,
            root,
            root_frame,
            physical_end,
            physical_bits,
        })
    }

    pub fn tables_needed(&self, physical: u64) -> Result<usize, PagingError> {
        let location = self.locate(physical)?;
        validate_huge(&location)?;
        Ok(location.level.saturating_sub(1))
    }

    pub fn frame_permissions(&self, physical: u64) -> Result<PagePermissions, PagingError> {
        let location = self.locate(physical)?;
        Ok(PagePermissions {
            writable: location.writable,
            executable: location.executable,
        })
    }

    /// # Safety
    /// New table frames are distinct exclusively owned RAM. Retain every used
    /// frame for kernel lifetime, since all child roots share these tables.
    /// The target is an owned managed RAM frame, not a page-table frame.
    /// ShootdownPending can follow publication; retain ALL supplied frames on
    /// that error, because the shared tree may already reference them.
    pub unsafe fn prepare_frame(
        &mut self,
        physical: u64,
        new_tables: &[u64],
    ) -> Result<usize, PagingError> {
        let mut location = self.locate(physical)?;
        validate_huge(&location)?;
        let count = location.level.saturating_sub(1);
        validate_new_tables(
            new_tables,
            count,
            physical,
            &location.ancestors[..5 - location.level],
            self.physical_bits,
        )?;
        let mut tables = [NonNull::dangling(); 2];
        for index in 0..count {
            tables[index] = self.direct_map.table_address(new_tables[index])?;
        }
        if count == 0 {
            return Ok(0);
        }

        let virtual_address = VirtAddr::new(self.direct_map.frame_address(physical)?);
        let parent_flags =
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
        if location.level == 3 {
            unsafe { split_table(tables[0], &location.entry, ONE_GIB, TWO_MIB) };
            let target = usize::from(virtual_address.p2_index());
            let middle = unsafe { tables[0].as_ref() }[target].clone();
            unsafe { split_table(tables[1], &middle, TWO_MIB, 4096) };
            unsafe {
                tables[0].as_mut()[target].set_addr(PhysAddr::new(new_tables[1]), parent_flags);
            }
        } else {
            unsafe { split_table(tables[0], &location.entry, TWO_MIB, 4096) };
        }
        compiler_fence(Ordering::Release);
        unsafe {
            location.table.as_mut()[location.index]
                .set_addr(PhysAddr::new(new_tables[0]), parent_flags);
        }
        flush_global()?;
        Ok(count)
    }

    /// # Safety
    /// The caller owns this frame and has ended incompatible references and
    /// mappings. Before clearing read_only, retire executable user mappings and
    /// complete their CPU/TLB lifetime. This changes the shared HHDM alias.
    pub unsafe fn protect_frame(
        &mut self,
        physical: u64,
        read_only: bool,
    ) -> Result<(), PagingError> {
        let mut location = self.locate(physical)?;
        if location.level != 1 {
            return Err(PagingError::InsufficientTables);
        }
        let entry = &mut unsafe { location.table.as_mut() }[location.index];
        let mut flags =
            (entry.flags() | PageTableFlags::NO_EXECUTE) & !PageTableFlags::USER_ACCESSIBLE;
        flags.set(PageTableFlags::WRITABLE, !read_only);
        entry.set_flags(flags);
        // Invalidate global translations as well as ordinary CR3-local entries.
        flush_global()?;
        Ok(())
    }

    fn locate(&self, physical: u64) -> Result<Alias, PagingError> {
        self.check_context()?;
        super::direct_map::physical_page(physical, self.physical_bits)?;
        if physical >= self.physical_end {
            return Err(PagingError::InvalidAddress);
        }
        let address = VirtAddr::new(self.direct_map.frame_address(physical)?);
        let indices = [
            usize::from(address.p4_index()),
            usize::from(address.p3_index()),
            usize::from(address.p2_index()),
            usize::from(address.p1_index()),
        ];
        let mut table = self.root;
        let mut ancestors = [0; 4];
        ancestors[0] = self.root_frame;
        let mut writable = true;
        let mut executable = true;
        for (depth, index) in indices.into_iter().enumerate() {
            let entry = unsafe { table.as_ref() }[index].clone();
            if !entry.flags().contains(PageTableFlags::PRESENT) {
                return Err(PagingError::NotMapped);
            }
            writable &= entry.flags().contains(PageTableFlags::WRITABLE);
            executable &= !entry.flags().contains(PageTableFlags::NO_EXECUTE);
            let level = 4 - depth;
            if level == 1 || entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                if level == 4 {
                    return Err(PagingError::UnsupportedPaging);
                }
                let size = match level {
                    3 => ONE_GIB,
                    2 => TWO_MIB,
                    _ => 4096,
                };
                let base = entry.addr().as_u64() & !(size - 1);
                if base + (address.as_u64() & (size - 1)) != physical {
                    return Err(PagingError::ContextChanged);
                }
                return Ok(Alias {
                    table,
                    index,
                    level,
                    entry,
                    ancestors,
                    writable,
                    executable,
                });
            }
            let next = entry.addr().as_u64();
            if ancestors[..=depth].contains(&next) {
                return Err(PagingError::AliasedFrame);
            }
            ancestors[depth + 1] = next;
            table = self.direct_map.table_address(next)?;
        }
        Err(PagingError::ContextChanged)
    }

    fn check_context(&self) -> Result<(), PagingError> {
        check_cpu()?;
        if Cr3::read().0.start_address().as_u64() != self.root_frame {
            return Err(PagingError::ContextChanged);
        }
        if !Cr0::read().contains(Cr0Flags::WRITE_PROTECT) {
            return Err(PagingError::UnsupportedPaging);
        }
        Ok(())
    }
}

fn validate_root(
    root: &PageTable,
    start: usize,
    end: usize,
    mmio_index: Option<usize>,
) -> Result<(), PagingError> {
    if mmio_index
        .is_some_and(|index| !(256..511).contains(&index) || (start..=end).contains(&index))
    {
        return Err(PagingError::UnsupportedPaging);
    }
    for (index, entry) in root.iter().enumerate() {
        if index != 511
            && !(start..=end).contains(&index)
            && Some(index) != mmio_index
            && !entry.is_unused()
        {
            return Err(PagingError::UnsupportedPaging);
        }
    }
    Ok(())
}

fn physical_page_bound(end: u64, bits: u8) -> Result<(), PagingError> {
    if !end.is_multiple_of(4096) {
        return Err(PagingError::UnalignedAddress);
    }
    super::direct_map::physical_page(end - 4096, bits)?;
    Ok(())
}

fn validate_huge(location: &Alias) -> Result<(), PagingError> {
    // A large-page PAT bit moves when split to 4 KiB entries. Reject this
    // unsupported memory type rather than silently changing cache semantics.
    if location.level > 1 && location.entry.addr().as_u64() & (1 << 12) != 0 {
        return Err(PagingError::UnsupportedPaging);
    }
    Ok(())
}

unsafe fn split_table(
    mut table: NonNull<PageTable>,
    entry: &PageTableEntry,
    old_size: u64,
    child_size: u64,
) {
    let base = entry.addr().as_u64() & !(old_size - 1);
    let mut flags = entry.flags() | PageTableFlags::NO_EXECUTE;
    flags.set(PageTableFlags::HUGE_PAGE, child_size != 4096);
    flags.remove(PageTableFlags::USER_ACCESSIBLE);
    for index in 0..512 {
        unsafe {
            table.as_mut()[index].set_addr(PhysAddr::new(base + index as u64 * child_size), flags);
        }
    }
}

pub(super) fn flush_global() -> Result<(), PagingError> {
    super::super::smp::global_tlb_flush().map_err(|_| PagingError::ShootdownPending)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_verified_additional_root_slot_is_allowed() {
        let mut root = PageTable::new();
        let flags = PageTableFlags::PRESENT | PageTableFlags::NO_EXECUTE;
        root[256].set_addr(PhysAddr::new(0x1000), flags);
        root[511].set_addr(PhysAddr::new(0x2000), flags);
        root[510].set_addr(PhysAddr::new(0x3000), flags);
        assert_eq!(
            validate_root(&root, 256, 256, None),
            Err(PagingError::UnsupportedPaging)
        );
        assert_eq!(validate_root(&root, 256, 256, Some(510)), Ok(()));
        for index in [0, 255, 256, 511, 512, usize::MAX] {
            assert_eq!(
                validate_root(&root, 256, 256, Some(index)),
                Err(PagingError::UnsupportedPaging)
            );
        }
        root[509].set_addr(PhysAddr::new(0x4000), flags);
        assert_eq!(
            validate_root(&root, 256, 256, Some(510)),
            Err(PagingError::UnsupportedPaging)
        );
    }

    #[test]
    fn huge_split_preserves_physical_extent_and_permissions() {
        let mut source = PageTableEntry::new();
        source.set_addr(
            PhysAddr::new(0x4000_0000),
            PageTableFlags::PRESENT
                | PageTableFlags::WRITABLE
                | PageTableFlags::HUGE_PAGE
                | PageTableFlags::GLOBAL,
        );
        let mut result = PageTable::new();
        unsafe { split_table(NonNull::from(&mut result), &source, ONE_GIB, TWO_MIB) };
        assert_eq!(result[0].addr().as_u64(), 0x4000_0000);
        assert_eq!(result[511].addr().as_u64(), 0x7fe0_0000);
        assert!(
            result[511]
                .flags()
                .contains(PageTableFlags::HUGE_PAGE | PageTableFlags::NO_EXECUTE)
        );
        assert!(
            result[0]
                .flags()
                .contains(PageTableFlags::GLOBAL | PageTableFlags::WRITABLE)
        );
        let mut leaf = PageTable::new();
        unsafe { split_table(NonNull::from(&mut leaf), &result[3], TWO_MIB, 4096) };
        assert_eq!(leaf[0].addr().as_u64(), 0x4060_0000);
        assert_eq!(leaf[511].addr().as_u64(), 0x407f_f000);
        assert!(
            !leaf[0]
                .flags()
                .intersects(PageTableFlags::HUGE_PAGE | PageTableFlags::USER_ACCESSIBLE)
        );
    }

    #[test]
    fn large_pat_entries_are_rejected_without_reinterpretation() {
        let mut entry = PageTableEntry::new();
        entry.set_addr(
            PhysAddr::new(0x4000_1000),
            PageTableFlags::PRESENT | PageTableFlags::HUGE_PAGE,
        );
        let location = Alias {
            table: NonNull::dangling(),
            index: 0,
            level: 3,
            entry,
            ancestors: [0; 4],
            writable: true,
            executable: true,
        };
        assert_eq!(
            validate_huge(&location),
            Err(PagingError::UnsupportedPaging)
        );
        assert_eq!(
            physical_page_bound(0x1001, 36),
            Err(PagingError::UnalignedAddress)
        );
        assert!(physical_page_bound(0x1_0000_0000, 36).is_ok());
    }
}
