use core::ptr::NonNull;
use x86_64::{PhysAddr, VirtAddr};

use super::{PAGE_SIZE, PagingError};

#[derive(Clone, Copy)]
pub struct DirectMap {
    offset: u64,
}

impl DirectMap {
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// # Safety
    /// RAM passed to this map must remain mapped at the supplied offset for
    /// every access. This object does not grant ownership of physical memory.
    pub unsafe fn new(offset: u64) -> Result<Self, PagingError> {
        kernel_page(offset)?;
        Ok(Self { offset })
    }

    pub fn frame_address(&self, physical: u64) -> Result<u64, PagingError> {
        physical_page(physical, 52)?;
        let start = self
            .offset
            .checked_add(physical)
            .ok_or(PagingError::AddressOverflow)?;
        kernel_page(start)?;
        Ok(start)
    }

    /// # Safety
    /// The caller exclusively owns this mapped RAM frame and has no live
    /// references or device accesses to its contents during initialization.
    pub unsafe fn zero_frame(&self, physical: u64) -> Result<(), PagingError> {
        let address = self.frame_address(physical)?;
        unsafe { (address as *mut u8).write_bytes(0, PAGE_SIZE as usize) };
        Ok(())
    }

    /// # Safety
    /// The frame is mapped RAM, initialized, and cannot be modified while read.
    pub unsafe fn frame_is_zero(&self, physical: u64) -> Result<bool, PagingError> {
        let address = self.frame_address(physical)?;
        let bytes = address as *const u8;
        for offset in 0..PAGE_SIZE as usize {
            if unsafe { bytes.add(offset).read_volatile() } != 0 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// # Safety
    /// The caller owns writable initialized RAM without incompatible aliases.
    /// The source must not overlap the destination frame.
    pub unsafe fn write_frame(
        &self,
        physical: u64,
        offset: usize,
        bytes: &[u8],
    ) -> Result<(), PagingError> {
        check_bounds(offset, bytes.len())?;
        let address = self.frame_address(physical)?;
        if bytes.is_empty() {
            return Ok(());
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                (address as *mut u8).add(offset),
                bytes.len(),
            )
        };
        Ok(())
    }

    /// # Safety
    /// The frame is initialized mapped RAM and cannot change while read.
    /// The output must not overlap the source frame.
    pub unsafe fn read_frame(
        &self,
        physical: u64,
        offset: usize,
        output: &mut [u8],
    ) -> Result<(), PagingError> {
        check_bounds(offset, output.len())?;
        let address = self.frame_address(physical)?;
        if output.is_empty() {
            return Ok(());
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                (address as *const u8).add(offset),
                output.as_mut_ptr(),
                output.len(),
            )
        };
        Ok(())
    }

    pub(super) fn table_address(
        &self,
        physical: u64,
    ) -> Result<NonNull<x86_64::structures::paging::PageTable>, PagingError> {
        let address = self.frame_address(physical)?;
        NonNull::new(address as *mut _).ok_or(PagingError::InvalidAddress)
    }
}

fn check_bounds(offset: usize, length: usize) -> Result<(), PagingError> {
    match offset.checked_add(length) {
        Some(end) if end <= PAGE_SIZE as usize => Ok(()),
        _ => Err(PagingError::InvalidAddress),
    }
}

pub(super) fn physical_page(address: u64, bits: u8) -> Result<PhysAddr, PagingError> {
    if !address.is_multiple_of(PAGE_SIZE) {
        return Err(PagingError::UnalignedAddress);
    }
    let end = address
        .checked_add(PAGE_SIZE - 1)
        .ok_or(PagingError::AddressOverflow)?;
    if bits > 52 || bits == 0 || end >> bits != 0 {
        return Err(PagingError::InvalidAddress);
    }
    PhysAddr::try_new(address).map_err(|_| PagingError::InvalidAddress)
}

pub(super) fn kernel_page(address: u64) -> Result<VirtAddr, PagingError> {
    if !address.is_multiple_of(PAGE_SIZE) {
        return Err(PagingError::UnalignedAddress);
    }
    let end = address
        .checked_add(PAGE_SIZE - 1)
        .ok_or(PagingError::AddressOverflow)?;
    if address < 0xffff_8000_0000_0000 {
        return Err(PagingError::InvalidAddress);
    }
    VirtAddr::try_new(end).map_err(|_| PagingError::InvalidAddress)?;
    VirtAddr::try_new(address).map_err(|_| PagingError::InvalidAddress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_copy_bounds_reject_overflow_and_cross_page_access() {
        assert_eq!(check_bounds(4095, 1), Ok(()));
        assert_eq!(check_bounds(4096, 0), Ok(()));
        assert_eq!(check_bounds(4096, 1), Err(PagingError::InvalidAddress));
        assert_eq!(
            check_bounds(usize::MAX, 2),
            Err(PagingError::InvalidAddress)
        );
    }

    #[test]
    fn direct_map_checks_full_frame() {
        let map = DirectMap {
            offset: 0xffff_8000_0000_0000,
        };
        assert_eq!(map.frame_address(0x1234_5000), Ok(0xffff_8000_1234_5000));
        assert_eq!(map.frame_address(1), Err(PagingError::UnalignedAddress));
        assert_eq!(map.frame_address(1 << 52), Err(PagingError::InvalidAddress));
        assert_eq!(
            map.frame_address(1 << 47),
            Err(PagingError::AddressOverflow)
        );
    }

    #[test]
    fn last_kernel_page_fits_and_addition_cannot_wrap() {
        let map = DirectMap {
            offset: 0xffff_ffff_ffff_f000,
        };
        assert_eq!(map.frame_address(0), Ok(0xffff_ffff_ffff_f000));
        assert_eq!(map.frame_address(0x1000), Err(PagingError::AddressOverflow));
        assert_eq!(kernel_page(0x1000), Err(PagingError::InvalidAddress));
        assert_eq!(
            kernel_page(0xffff_8000_0000_0001),
            Err(PagingError::UnalignedAddress)
        );
    }

    #[test]
    fn physical_width_and_alignment_are_checked() {
        assert_eq!(
            physical_page((1 << 36) - PAGE_SIZE, 36).unwrap().as_u64(),
            (1 << 36) - PAGE_SIZE
        );
        assert_eq!(physical_page(1 << 36, 36), Err(PagingError::InvalidAddress));
        assert_eq!(
            physical_page(0x1001, 36),
            Err(PagingError::UnalignedAddress)
        );
        assert_eq!(physical_page(0, 64), Err(PagingError::InvalidAddress));
    }
}
