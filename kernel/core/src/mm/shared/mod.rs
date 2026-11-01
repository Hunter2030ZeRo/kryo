use crate::mm::frame::{FRAME_SIZE, FrameAllocError, OwnedFrame};
use crate::mm::vm::FrameAllocator;

#[derive(Debug, PartialEq, Eq)]
pub enum SharedError<E> {
    InvalidSize,
    Allocation(FrameAllocError),
    Backend(E),
}

/// Initializes shared RAM before it is published through an object or mapping.
///
/// # Safety
/// Success zeroes the complete frame and establishes a persistent writable,
/// non-executable kernel alias. Failure leaves the frame unpublished and safe
/// to return to its allocator, without any outstanding accesses or references.
/// Shared frames never acquire executable aliases during their lifetime.
pub unsafe trait SharedBackend {
    type Error;

    /// # Safety
    /// physical is an exclusively owned, unpublished RAM frame.
    unsafe fn initialize_shared(&mut self, physical: u64) -> Result<(), Self::Error>;
}

/// Backing ownership for a bounded, non-executable shared-memory object.
/// Keep this payload immutable in the object arena. Mappings retain strong
/// references to that object until their PTEs and TLB access are retired.
pub struct SharedMemory<'pool, F: FrameAllocator, const PAGES: usize> {
    allocator: &'pool F,
    frames: [Option<OwnedFrame<'pool>>; PAGES],
    page_count: usize,
}

impl<'pool, F: FrameAllocator, const PAGES: usize> SharedMemory<'pool, F, PAGES> {
    pub fn new<B: SharedBackend>(
        allocator: &'pool F,
        page_count: usize,
        backend: &mut B,
    ) -> Result<Self, SharedError<B::Error>> {
        if page_count == 0
            || page_count > PAGES
            || u64::try_from(page_count)
                .ok()
                .and_then(|pages| pages.checked_mul(FRAME_SIZE))
                .is_none()
        {
            return Err(SharedError::InvalidSize);
        }
        let mut memory = Self {
            allocator,
            frames: [const { None }; PAGES],
            page_count,
        };
        for slot in &mut memory.frames[..page_count] {
            let frame = allocator.allocate().map_err(SharedError::Allocation)?;
            let physical = frame.address();
            *slot = Some(frame);
            // The unpublished owner is retained even when initialization fails.
            unsafe { backend.initialize_shared(physical) }.map_err(SharedError::Backend)?;
        }
        Ok(memory)
    }

    pub const fn page_count(&self) -> usize {
        self.page_count
    }

    pub const fn byte_len(&self) -> u64 {
        self.page_count as u64 * FRAME_SIZE
    }

    /// Describes a page; the address does not grant ownership or mapping authority.
    pub fn page_address(&self, index: usize) -> Option<u64> {
        self.frames.get(index)?.as_ref().map(OwnedFrame::address)
    }
}

impl<F: FrameAllocator, const PAGES: usize> Drop for SharedMemory<'_, F, PAGES> {
    fn drop(&mut self) {
        for frame in &mut self.frames {
            if let Some(frame) = frame.take() {
                // The last object reference is gone. Every mapping lease must
                // already have retired user access; the RW+NX alias stays fixed.
                if unsafe { self.allocator.release(frame) }.is_err() {
                    panic!("Shared frame ownership invariant");
                }
            }
        }
    }
}

/// A strong page lease retained by a user mapping.
///
/// # Safety
/// physical returns the same page-aligned, initialized RAM frame throughout
/// this value's lifetime. Keep its backing object immutable and strongly referenced. Its
/// kernel alias remains RW+NX, no executable mapping may exist, and dropping
/// this lease must not release backing still retained by another lease.
/// An arena must not drop payloads whose strong references were forgotten.
pub unsafe trait SharedPage {
    fn physical(&self) -> u64;
}

/// Default backing type for address spaces that only map private pages.
pub enum NoSharedPage {}

unsafe impl SharedPage for NoSharedPage {
    fn physical(&self) -> u64 {
        match *self {}
    }
}
