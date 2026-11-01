use alloc::vec::Vec;
use core::ptr::NonNull;
use core::sync::atomic::{Ordering, compiler_fence};
use x86_64::registers::control::Cr3;
use x86_64::registers::model_specific::Msr;
use x86_64::structures::paging::{PageTable, PageTableFlags, page_table::PageTableEntry};
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

struct RegionLeaf {
    table: NonNull<PageTable>,
    frame: u64,
    retiring: [Option<u64>; PAGE_COUNT],
}

/// A shared supervisor branch for kernel stacks and MMIO mappings. Additional
/// leaf tables extend it within one pre-linked page directory (up to 1 GiB).
pub struct KernelRegion {
    root: NonNull<PageTable>,
    leaf: NonNull<PageTable>,
    middle: NonNull<PageTable>,
    direct: DirectMap,
    extra: Vec<RegionLeaf>,
    extending: bool,
    root_frame: u64,
    tables: [u64; 3],
    base: VirtAddr,
    root_index: usize,
    physical_bits: u8,
    attached: bool,
    detaching: bool,
    retiring: [Option<u64>; PAGE_COUNT],
}

impl KernelRegion {
    /// # Safety
    /// Run on the BSP with IRQs disabled and exclusive page-table access.
    /// Retain the three exclusively owned RAM table frames until detach.
    /// Exclusively owned kernel-image table frames may be attached during early
    /// boot. Establish supervisor/HHDM NX and W^X policy before cloning user
    /// roots or executing users. All such roots must be retired before detach.
    /// On ShootdownPending the branch may already be linked; retain every
    /// supplied table owner and complete a global flush before reusing them.
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
        super::kernel_map::flush_global()?;
        Ok(Self {
            root,
            leaf: tables[2],
            middle: tables[1],
            direct: direct_map,
            extra: Vec::new(),
            extending: false,
            root_frame,
            tables: table_frames,
            base,
            root_index,
            physical_bits: bits,
            attached: true,
            detaching: false,
            retiring: [None; PAGE_COUNT],
        })
    }

    pub fn address(&self, slot: usize) -> Result<u64, PagingError> {
        region_address(self.base.as_u64(), slot, self.page_capacity())
    }

    pub fn page_capacity(&self) -> usize {
        (1 + self.extra.len()) * PAGE_COUNT
    }

    pub fn can_extend(&self) -> bool {
        usize::from(self.base.p2_index()) + 1 + self.extra.len() < PAGE_COUNT
    }

    /// # Safety
    /// The caller exclusively owns and retains this zeroable RAM frame until
    /// detach succeeds. Every cloned root shares this existing page directory;
    /// no CPU may own a user lease during publication. On ShootdownPending the
    /// new table is already linked and retained: retry_extension must finish
    /// before mapping pages or dispatching users. Other errors do not link it.
    pub unsafe fn extend(&mut self, frame: u64) -> Result<(), PagingError> {
        self.check_context()?;
        self.check_ready()?;
        if !self.can_extend() {
            return Err(PagingError::InsufficientTables);
        }
        physical_page(frame, self.physical_bits)?;
        if self.is_table(frame) || self.aliases(frame) {
            return Err(PagingError::AliasedFrame);
        }
        let index = usize::from(self.base.p2_index()) + 1 + self.extra.len();
        if !unsafe { self.middle.as_ref() }[index].is_unused() {
            return Err(PagingError::SlotOccupied);
        }
        let table = self.direct.table_address(frame)?;
        unsafe { self.publish_leaf(index, table, frame, super::kernel_map::flush_global) }
    }

    pub fn retry_extension(&mut self) -> Result<(), PagingError> {
        self.check_context()?;
        finish_extension(&mut self.extending, super::kernel_map::flush_global)
    }

    unsafe fn publish_leaf(
        &mut self,
        index: usize,
        table: NonNull<PageTable>,
        frame: u64,
        flush: impl FnOnce() -> Result<(), PagingError>,
    ) -> Result<(), PagingError> {
        self.extra
            .try_reserve(1)
            .map_err(|_| PagingError::InsufficientTables)?;
        unsafe { table.as_ptr().write(PageTable::new()) };
        // Retain the software record before publishing the parent link. A
        // failed flush must never forget an attached table's physical identity.
        self.extra.push(RegionLeaf {
            table,
            frame,
            retiring: [None; PAGE_COUNT],
        });
        self.extending = true;
        compiler_fence(Ordering::Release);
        (unsafe { self.middle.as_mut() })[index].set_addr(
            PhysAddr::new(frame),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
        );
        finish_extension(&mut self.extending, flush)
    }

    pub(super) fn mmio_root_index(&self, direct_map: DirectMap) -> Result<usize, PagingError> {
        self.check_context()?;
        self.check_ready()?;
        if !self.extra.is_empty() {
            return Err(PagingError::UnsupportedPaging);
        }
        if self.has_retiring() {
            return Err(PagingError::ShootdownPending);
        }
        if direct_map.table_address(self.root_frame)? != self.root
            || direct_map.table_address(self.tables[2])? != self.leaf
        {
            return Err(PagingError::ContextChanged);
        }
        check_uncached_pat()?;
        validate_parent_entry(
            &unsafe { self.root.as_ref() }[self.root_index],
            self.tables[0],
        )?;
        let directory = direct_map.table_address(self.tables[0])?;
        let middle = direct_map.table_address(self.tables[1])?;
        validate_parent_table(
            unsafe { directory.as_ref() },
            usize::from(self.base.p3_index()),
            self.tables[1],
        )?;
        validate_parent_table(
            unsafe { middle.as_ref() },
            usize::from(self.base.p2_index()),
            self.tables[2],
        )?;
        validate_mmio_leaves(unsafe { self.leaf.as_ref() })?;
        Ok(self.root_index)
    }

    /// # Safety
    /// The caller exclusively owns this RAM frame and retains it until unmap
    /// completes. The mapping must not conflict with existing Rust references.
    pub unsafe fn map_ram(&mut self, slot: usize, physical: u64) -> Result<u64, PagingError> {
        unsafe { self.map(slot, physical, false, true) }
    }

    /// # Safety
    /// The caller retains authority over reserved firmware RAM through unmap.
    /// Its aliases must use compatible write-back memory types. No mutable Rust
    /// references or concurrent firmware writes may cover bytes read here. This
    /// borrowed mapping grants no allocation ownership or permission to reclaim.
    pub unsafe fn map_firmware(&mut self, slot: usize, physical: u64) -> Result<u64, PagingError> {
        unsafe { self.map(slot, physical, false, false) }
    }

    /// # Safety
    /// The caller has authority over this MMIO page and retains that authority
    /// until unmap. Existing aliases must have compatible memory types.
    /// This page is device memory, never Usable RAM or a frame-pool allocation.
    /// Do not create ordinary RAM references through this alias.
    pub unsafe fn map_mmio(&mut self, slot: usize, physical: u64) -> Result<u64, PagingError> {
        check_uncached_pat()?;
        unsafe { self.map(slot, physical, true, true) }
    }

    pub fn permissions(&self, slot: usize) -> Result<RegionPermissions, PagingError> {
        self.check_context()?;
        self.check_ready()?;
        self.address(slot)?;
        let (table, retiring) = self.page(slot);
        if retiring.is_some() {
            return Err(PagingError::ShootdownPending);
        }
        let entry = &unsafe { table.as_ref() }[slot % PAGE_COUNT];
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
        self.check_ready()?;
        self.address(slot)?;
        let (mut table, retiring) = self.page_mut(slot);
        let entry = &mut unsafe { table.as_mut() }[slot % PAGE_COUNT];
        retire_entry(entry, retiring, super::kernel_map::flush_global)
    }

    /// # Safety
    /// Every user root sharing this branch has been retired. No CPU uses the
    /// branch, and all aliases have been unmapped before table ownership ends.
    pub unsafe fn detach(&mut self) -> Result<(), PagingError> {
        self.check_context()?;
        self.retry_extension()?;
        if self.has_retiring() {
            return Err(PagingError::ShootdownPending);
        }
        if self.leaves().any(|table| {
            unsafe { table.as_ref() }
                .iter()
                .any(|entry| !entry.is_unused())
        }) {
            return Err(PagingError::SlotOccupied);
        }
        retire_branch(
            &mut unsafe { self.root.as_mut() }[self.root_index],
            &mut self.detaching,
            super::kernel_map::flush_global,
        )?;
        self.attached = false;
        self.extra = Vec::new();
        Ok(())
    }

    unsafe fn map(
        &mut self,
        slot: usize,
        physical: u64,
        uncached: bool,
        writable: bool,
    ) -> Result<u64, PagingError> {
        self.check_context()?;
        self.check_ready()?;
        let address = self.address(slot)?;
        let (_, retiring) = self.page(slot);
        if retiring.is_some() {
            return Err(PagingError::ShootdownPending);
        }
        let frame = physical_page(physical, self.physical_bits)?;
        let (mut table, _) = self.page_mut(slot);
        if !unsafe { table.as_ref() }[slot % PAGE_COUNT].is_unused() {
            return Err(PagingError::AlreadyMapped);
        }
        if self.is_table(physical) || self.aliases(physical) {
            return Err(PagingError::AliasedFrame);
        }
        (unsafe { table.as_mut() })[slot % PAGE_COUNT]
            .set_addr(frame, leaf_flags(uncached, writable));
        super::kernel_map::flush_global()?;
        Ok(address)
    }

    fn is_table(&self, physical: u64) -> bool {
        physical == self.root_frame
            || self.tables.contains(&physical)
            || self.extra.iter().any(|leaf| leaf.frame == physical)
    }

    fn leaves(&self) -> impl Iterator<Item = NonNull<PageTable>> + '_ {
        core::iter::once(self.leaf).chain(self.extra.iter().map(|leaf| leaf.table))
    }

    fn aliases(&self, physical: u64) -> bool {
        self.retiring.iter().flatten().any(|old| *old == physical)
            || self
                .extra
                .iter()
                .any(|leaf| leaf.retiring.iter().flatten().any(|old| *old == physical))
            || self.leaves().any(|table| {
                unsafe { table.as_ref() }.iter().any(|entry| {
                    entry.flags().contains(PageTableFlags::PRESENT)
                        && entry.addr().as_u64() == physical
                })
            })
    }

    fn has_retiring(&self) -> bool {
        self.retiring.iter().any(Option::is_some)
            || self
                .extra
                .iter()
                .any(|leaf| leaf.retiring.iter().any(Option::is_some))
    }

    fn page(&self, slot: usize) -> (NonNull<PageTable>, &Option<u64>) {
        let (leaf, entry) = (slot / PAGE_COUNT, slot % PAGE_COUNT);
        if leaf == 0 {
            (self.leaf, &self.retiring[entry])
        } else {
            let leaf = &self.extra[leaf - 1];
            (leaf.table, &leaf.retiring[entry])
        }
    }

    fn page_mut(&mut self, slot: usize) -> (NonNull<PageTable>, &mut Option<u64>) {
        let (leaf, entry) = (slot / PAGE_COUNT, slot % PAGE_COUNT);
        if leaf == 0 {
            (self.leaf, &mut self.retiring[entry])
        } else {
            let leaf = &mut self.extra[leaf - 1];
            (leaf.table, &mut leaf.retiring[entry])
        }
    }

    fn check_ready(&self) -> Result<(), PagingError> {
        if self.detaching {
            return Err(PagingError::ContextChanged);
        }
        if self.extending {
            return Err(PagingError::ShootdownPending);
        }
        Ok(())
    }

    fn check_context(&self) -> Result<(), PagingError> {
        check_cpu()?;
        if !self.attached || Cr3::read().0.start_address().as_u64() != self.root_frame {
            return Err(PagingError::ContextChanged);
        }
        let entry = &unsafe { self.root.as_ref() }[self.root_index];
        if self.detaching {
            return if entry.is_unused() {
                Ok(())
            } else {
                Err(PagingError::ContextChanged)
            };
        }
        if !entry.flags().contains(PageTableFlags::PRESENT)
            || entry.addr().as_u64() != self.tables[0]
        {
            return Err(PagingError::ContextChanged);
        }
        Ok(())
    }
}

fn validate_parent_entry(entry: &PageTableEntry, physical: u64) -> Result<(), PagingError> {
    let flags = entry.flags();
    if !flags.contains(PageTableFlags::PRESENT | PageTableFlags::NO_EXECUTE)
        || flags.intersects(PageTableFlags::USER_ACCESSIBLE | PageTableFlags::HUGE_PAGE)
        || entry.addr().as_u64() != physical
    {
        return Err(PagingError::ContextChanged);
    }
    Ok(())
}

fn validate_parent_table(
    table: &PageTable,
    selected: usize,
    physical: u64,
) -> Result<(), PagingError> {
    validate_parent_entry(&table[selected], physical)?;
    if table
        .iter()
        .enumerate()
        .any(|(index, entry)| index != selected && !entry.is_unused())
    {
        return Err(PagingError::UnsupportedPaging);
    }
    Ok(())
}

fn validate_mmio_leaves(table: &PageTable) -> Result<(), PagingError> {
    let required = PageTableFlags::PRESENT
        | PageTableFlags::NO_EXECUTE
        | PageTableFlags::NO_CACHE
        | PageTableFlags::WRITE_THROUGH;
    for entry in table.iter().filter(|entry| !entry.is_unused()) {
        if !entry.flags().contains(required)
            || entry
                .flags()
                .intersects(PageTableFlags::USER_ACCESSIBLE | PageTableFlags::HUGE_PAGE)
        {
            // At the leaf, bit 7 selects PAT rather than a huge page. It must
            // remain clear so PCD/PWT select the validated UC entry 3.
            return Err(PagingError::InvalidPermissions);
        }
    }
    Ok(())
}

fn finish_extension(
    pending: &mut bool,
    flush: impl FnOnce() -> Result<(), PagingError>,
) -> Result<(), PagingError> {
    if *pending {
        flush()?;
        *pending = false;
    }
    Ok(())
}

fn retire_branch(
    entry: &mut PageTableEntry,
    detaching: &mut bool,
    flush: impl FnOnce() -> Result<(), PagingError>,
) -> Result<(), PagingError> {
    *detaching = true;
    entry.set_unused();
    flush()?;
    *detaching = false;
    Ok(())
}

fn retire_entry(
    entry: &mut PageTableEntry,
    retiring: &mut Option<u64>,
    flush: impl FnOnce() -> Result<(), PagingError>,
) -> Result<u64, PagingError> {
    let physical = if let Some(physical) = *retiring {
        physical
    } else {
        if !entry.flags().contains(PageTableFlags::PRESENT) {
            return Err(PagingError::NotMapped);
        }
        let physical = entry.addr().as_u64();
        *retiring = Some(physical);
        entry.set_unused();
        physical
    };
    flush()?;
    *retiring = None;
    Ok(physical)
}

fn region_base(address: u64) -> Result<VirtAddr, PagingError> {
    let address = kernel_page(address)?;
    if !address.as_u64().is_multiple_of(REGION_SIZE) || usize::from(address.p4_index()) == 511 {
        return Err(PagingError::InvalidAddress);
    }
    region_address(address.as_u64(), PAGE_COUNT - 1, PAGE_COUNT)?;
    Ok(address)
}

fn region_address(base: u64, slot: usize, capacity: usize) -> Result<u64, PagingError> {
    if slot >= capacity {
        return Err(PagingError::InvalidAddress);
    }
    let address = base
        .checked_add(slot as u64 * 4096)
        .ok_or(PagingError::AddressOverflow)?;
    kernel_page(address)?;
    Ok(address)
}

fn leaf_flags(uncached: bool, writable: bool) -> PageTableFlags {
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::NO_EXECUTE;
    if writable {
        flags |= PageTableFlags::WRITABLE;
    }
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
    fn initial_region_may_contain_only_supervisor_nx_uc_device_leaves() {
        let mut leaves = PageTable::new();
        leaves[0].set_addr(PhysAddr::new(0xfee0_0000), leaf_flags(true, true));
        leaves[12].set_addr(PhysAddr::new(0xfed0_0000), leaf_flags(true, false));
        assert_eq!(validate_mmio_leaves(&leaves), Ok(()));
        let flags = leaves[0].flags();
        for bad in [
            flags & !PageTableFlags::PRESENT,
            flags & !PageTableFlags::NO_EXECUTE,
            flags & !PageTableFlags::NO_CACHE,
            flags & !PageTableFlags::WRITE_THROUGH,
            flags | PageTableFlags::USER_ACCESSIBLE,
            flags | PageTableFlags::HUGE_PAGE,
            leaf_flags(false, true),
        ] {
            leaves[0].set_flags(bad);
            assert_eq!(
                validate_mmio_leaves(&leaves),
                Err(PagingError::InvalidPermissions)
            );
        }
    }

    #[test]
    fn initial_region_parent_links_are_exact_and_have_no_extra_branches() {
        let mut table = PageTable::new();
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
        table[7].set_addr(PhysAddr::new(0x3000), flags);
        assert_eq!(validate_parent_table(&table, 7, 0x3000), Ok(()));
        assert_eq!(
            validate_parent_table(&table, 7, 0x4000),
            Err(PagingError::ContextChanged)
        );
        for bad in [
            flags & !PageTableFlags::NO_EXECUTE,
            flags | PageTableFlags::USER_ACCESSIBLE,
            flags | PageTableFlags::HUGE_PAGE,
        ] {
            table[7].set_flags(bad);
            assert_eq!(
                validate_parent_table(&table, 7, 0x3000),
                Err(PagingError::ContextChanged)
            );
        }
        table[7].set_flags(flags);
        table[8].set_addr(PhysAddr::new(0x4000), flags);
        assert_eq!(
            validate_parent_table(&table, 7, 0x3000),
            Err(PagingError::UnsupportedPaging)
        );
    }

    #[test]
    fn unmap_keeps_physical_identity_until_every_flush_acknowledges() {
        let mut entry = PageTableEntry::new();
        entry.set_addr(PhysAddr::new(0x4000), PageTableFlags::PRESENT);
        let mut retiring = None;
        for _ in 0..2 {
            assert_eq!(
                retire_entry(&mut entry, &mut retiring, || Err(
                    PagingError::ShootdownPending
                )),
                Err(PagingError::ShootdownPending)
            );
            assert!(entry.is_unused());
            assert_eq!(retiring, Some(0x4000));
        }
        assert_eq!(
            retire_entry(&mut entry, &mut retiring, || Ok(())),
            Ok(0x4000)
        );
        assert_eq!(retiring, None);
        assert_eq!(
            retire_entry(&mut entry, &mut retiring, || panic!(
                "Missing mapping must not flush"
            )),
            Err(PagingError::NotMapped)
        );
    }

    #[test]
    fn parent_retirement_remains_retryable_after_link_was_removed() {
        let mut entry = PageTableEntry::new();
        entry.set_addr(
            PhysAddr::new(0x8000),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
        );
        let mut detaching = false;
        assert_eq!(
            retire_branch(&mut entry, &mut detaching, || Err(
                PagingError::ShootdownPending
            )),
            Err(PagingError::ShootdownPending)
        );
        assert!(entry.is_unused() && detaching);
        assert_eq!(retire_branch(&mut entry, &mut detaching, || Ok(())), Ok(()));
        assert!(entry.is_unused() && !detaching);
    }

    #[test]
    fn region_bounds_leave_room_for_guards_and_reject_image_slot() {
        let base = 0xffff_ff00_0000_0000;
        assert!(region_base(base).is_ok());
        assert_eq!(region_address(base, 511, PAGE_COUNT), Ok(base + 511 * 4096));
        assert_eq!(
            region_address(base, 512, PAGE_COUNT),
            Err(PagingError::InvalidAddress)
        );
        assert_eq!(region_base(base + 4096), Err(PagingError::InvalidAddress));
        assert_eq!(
            region_base(0xffff_ffff_8000_0000),
            Err(PagingError::InvalidAddress)
        );
    }

    #[test]
    fn additional_leaf_retains_identity_through_a_missing_acknowledgement() {
        use alloc::boxed::Box;
        let mut root = Box::new(PageTable::new());
        let mut middle = Box::new(PageTable::new());
        let mut first = Box::new(PageTable::new());
        let mut added = Box::new(PageTable::new());
        let base = VirtAddr::new(0xffff_ff00_0000_0000);
        let mut region = KernelRegion {
            root: NonNull::from(root.as_mut()),
            leaf: NonNull::from(first.as_mut()),
            middle: NonNull::from(middle.as_mut()),
            direct: unsafe { DirectMap::new(0xffff_8000_0000_0000) }.unwrap(),
            extra: Vec::new(),
            extending: false,
            root_frame: 0x1000,
            tables: [0x2000, 0x3000, 0x4000],
            base,
            root_index: usize::from(base.p4_index()),
            physical_bits: 52,
            attached: true,
            detaching: false,
            retiring: [None; PAGE_COUNT],
        };
        assert_eq!(region.page_capacity(), PAGE_COUNT);
        assert!(region.can_extend());
        let table = NonNull::from(added.as_mut());
        assert_eq!(
            unsafe { region.publish_leaf(1, table, 0x5000, || Err(PagingError::ShootdownPending)) },
            Err(PagingError::ShootdownPending)
        );
        assert_eq!(region.page_capacity(), 2 * PAGE_COUNT);
        assert_eq!(region.extra[0].frame, 0x5000);
        assert_eq!(region.extra[0].table, table);
        assert_eq!(middle[1].addr().as_u64(), 0x5000);
        assert_eq!(region.check_ready(), Err(PagingError::ShootdownPending));
        assert!(region.is_table(0x5000));
        assert_eq!(
            finish_extension(&mut region.extending, || Err(PagingError::ShootdownPending)),
            Err(PagingError::ShootdownPending)
        );
        assert!(region.extending);
        assert_eq!(finish_extension(&mut region.extending, || Ok(())), Ok(()));
        assert_eq!(region.check_ready(), Ok(()));
        assert_eq!(region.address(511), Ok(base.as_u64() + 511 * 4096));
        assert_eq!(region.address(512), Ok(base.as_u64() + 512 * 4096));
        assert_eq!(region.address(1023), Ok(base.as_u64() + 1023 * 4096));
        assert_eq!(region.address(1024), Err(PagingError::InvalidAddress));
        // The first and additional leaf have independent retirement records.
        *region.page_mut(511).1 = Some(0x8000);
        *region.page_mut(512).1 = Some(0x9000);
        assert_eq!(*region.page(511).1, Some(0x8000));
        assert_eq!(*region.page(512).1, Some(0x9000));
        assert!(region.aliases(0x8000) && region.aliases(0x9000));
        added[511].set_addr(PhysAddr::new(0xa000), leaf_flags(false, true));
        assert!(region.aliases(0xa000));
        assert!(!region.aliases(0xb000));
        assert!(region.has_retiring());
        *region.page_mut(511).1 = None;
        *region.page_mut(512).1 = None;
        assert!(!region.has_retiring());
    }

    #[test]
    fn ram_and_mmio_are_supervisor_writable_nx() {
        let ram = decode_permissions(leaf_flags(false, true));
        assert!(ram.writable && !ram.executable && !ram.user && !ram.uncached);
        let mmio = decode_permissions(leaf_flags(true, true));
        assert!(mmio.writable && !mmio.executable && !mmio.user && mmio.uncached);
        assert!(!leaf_flags(true, true).contains(PageTableFlags::GLOBAL));
        let firmware = decode_permissions(leaf_flags(false, false));
        assert!(!firmware.writable && !firmware.executable && !firmware.user && !firmware.uncached);
    }
}
