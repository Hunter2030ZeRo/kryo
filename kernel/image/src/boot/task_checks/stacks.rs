use super::super::memory::BootFramePool;
use kryo_core::mm::frame::OwnedFrame;
use kryo_hal::{DirectMap, KernelRegion, PagingError};

pub(super) struct KernelStack {
    frames: [Option<OwnedFrame<'static>>; 4],
    first_slot: usize,
    top: u64,
}

impl KernelStack {
    pub(super) fn new(
        pool: &'static BootFramePool,
        direct: DirectMap,
        region: &mut KernelRegion,
        first_slot: usize,
    ) -> Self {
        assert!(first_slot > 0);
        assert!(
            region.permissions(first_slot - 1).is_err(),
            "Kernel stack guard is mapped"
        );
        let mut frames = [const { None }; 4];
        for (index, slot) in frames.iter_mut().enumerate() {
            let frame = pool.allocate().expect("Cannot allocate kernel stack");
            // These newly owned frames are private until their supervisor mapping is installed.
            unsafe {
                direct
                    .zero_frame(frame.address())
                    .expect("Cannot initialize kernel stack");
                region
                    .map_ram(first_slot + index, frame.address())
                    .expect("Cannot map kernel stack");
            }
            let flags = region
                .permissions(first_slot + index)
                .expect("Missing kernel stack page");
            assert!(flags.writable && !flags.executable && !flags.user && !flags.uncached);
            *slot = Some(frame);
        }
        let top = region
            .address(first_slot + 3)
            .expect("Invalid kernel stack address")
            + 4096;
        Self {
            frames,
            first_slot,
            top,
        }
    }

    pub(super) const fn top(&self) -> u64 {
        self.top
    }

    /// The caller has returned to its boot stack and restored TSS.RSP0.
    pub(super) unsafe fn close(
        &mut self,
        pool: &BootFramePool,
        region: &mut KernelRegion,
    ) -> Result<(), PagingError> {
        for (index, slot) in self.frames.iter_mut().enumerate() {
            let Some(frame) = slot.as_ref() else { continue };
            // No CPU uses this entry stack. Unmap flushes before its frame is released.
            let physical = unsafe { region.unmap(self.first_slot + index) }?;
            assert_eq!(physical, frame.address());
            let frame = slot.take().unwrap();
            unsafe { pool.release(frame) }.expect("Cannot return kernel stack frame");
        }
        Ok(())
    }
}
