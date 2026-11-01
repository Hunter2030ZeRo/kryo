use crate::boot::memory::BootFramePool;
use crate::runtime::KernelRegion;
use kryo_core::mm::frame::{FrameAllocError, FrameFreeError, OwnedFrame};
use kryo_hal::{DirectMap, PagingError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StackError {
    Allocation(FrameAllocError),
    Paging(PagingError),
    Release(FrameFreeError),
    MappingChanged,
}

#[derive(Debug)]
pub(crate) struct StackBuildError {
    pub(crate) error: StackError,
    pub(crate) cleanup_error: Option<StackError>,
    pub(crate) retained: Option<KernelStack>,
}

/// Private entry-stack frames remain owned until explicit retirement succeeds.
#[derive(Debug)]
pub(crate) struct KernelStack {
    frames: [Option<OwnedFrame<'static>>; 4],
    mapped: [bool; 4],
    first_slot: usize,
    top: u64,
    quarantined: bool,
}

impl KernelStack {
    pub(crate) fn try_new(
        pool: &'static BootFramePool,
        direct: DirectMap,
        region: &mut KernelRegion,
        first_slot: usize,
    ) -> Result<Self, StackBuildError> {
        let mut stack = Self {
            frames: [const { None }; 4],
            mapped: [false; 4],
            first_slot,
            top: 0,
            quarantined: false,
        };
        let result = stack.initialize(pool, direct, region);
        if let Err(error) = result {
            // The unpublished stack has never been used by a CPU. Keep any
            // owner whose unmap or pool return still needs cleanup retry.
            let cleanup_error = unsafe { stack.close(pool, region) }.err();
            return Err(StackBuildError {
                error,
                retained: cleanup_error.map(|_| stack),
                cleanup_error,
            });
        }
        Ok(stack)
    }

    #[cfg(feature = "boot-tests")]
    pub(crate) fn new(
        pool: &'static BootFramePool,
        direct: DirectMap,
        region: &mut KernelRegion,
        first_slot: usize,
    ) -> Self {
        Self::try_new(pool, direct, region, first_slot)
            .expect("Cannot prepare fixture kernel stack")
    }

    fn initialize(
        &mut self,
        pool: &'static BootFramePool,
        direct: DirectMap,
        region: &mut KernelRegion,
    ) -> Result<(), StackError> {
        let guard = self
            .first_slot
            .checked_sub(1)
            .ok_or(StackError::Paging(PagingError::InvalidAddress))?;
        let last = self
            .first_slot
            .checked_add(3)
            .ok_or(StackError::Paging(PagingError::InvalidAddress))?;
        self.top = region
            .address(last)
            .map_err(StackError::Paging)?
            .checked_add(4096)
            .ok_or(StackError::Paging(PagingError::AddressOverflow))?;
        for slot in guard..=last {
            match region.permissions(slot) {
                Err(PagingError::NotMapped) => {}
                Err(error) => return Err(StackError::Paging(error)),
                Ok(_) => return Err(StackError::Paging(PagingError::AlreadyMapped)),
            }
        }
        for index in 0..self.frames.len() {
            let frame = pool.allocate().map_err(StackError::Allocation)?;
            let physical = frame.address();
            self.frames[index] = Some(frame);
            // The frame is retained privately before the first operation that
            // may publish an alias, so every failure keeps its owner reachable.
            unsafe {
                direct.zero_frame(physical).map_err(StackError::Paging)?;
                match region.map_ram(self.first_slot + index, physical) {
                    Ok(_) => {}
                    Err(PagingError::ShootdownPending) => {
                        self.mapped[index] = true;
                        return Err(StackError::Paging(PagingError::ShootdownPending));
                    }
                    Err(error) => return Err(StackError::Paging(error)),
                }
            }
            self.mapped[index] = true;
            let flags = region
                .permissions(self.first_slot + index)
                .map_err(StackError::Paging)?;
            if !flags.writable || flags.executable || flags.user || flags.uncached {
                return Err(StackError::Paging(PagingError::InvalidPermissions));
            }
        }
        Ok(())
    }

    pub(crate) const fn top(&self) -> u64 {
        self.top
    }

    /// # Safety
    /// No CPU may use this entry stack; restore the boot stack and TSS.RSP0
    /// before retiring a published stack. Retain this object on failure.
    pub(crate) unsafe fn close(
        &mut self,
        pool: &BootFramePool,
        region: &mut KernelRegion,
    ) -> Result<(), StackError> {
        if self.quarantined {
            return Err(StackError::MappingChanged);
        }
        for index in 0..self.frames.len() {
            let Some(frame) = self.frames[index].as_ref() else {
                continue;
            };
            if self.mapped[index] {
                let physical =
                    unsafe { region.unmap(self.first_slot + index) }.map_err(StackError::Paging)?;
                self.mapped[index] = false;
                if physical != frame.address() {
                    self.quarantined = true;
                    return Err(StackError::MappingChanged);
                }
            }
            let frame = self.frames[index].take().unwrap();
            if let Err((error, frame)) = unsafe { pool.release(frame) } {
                self.frames[index] = Some(frame);
                return Err(StackError::Release(error));
            }
        }
        Ok(())
    }
}
