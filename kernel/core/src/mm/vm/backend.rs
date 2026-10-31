use crate::mm::frame::{FrameAllocError, FrameFreeError, FramePool, OwnedFrame};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permissions {
    ReadOnly,
    ReadWrite,
    ReadExecute,
}

impl Permissions {
    pub const fn new(writable: bool, executable: bool) -> Option<Self> {
        match (writable, executable) {
            (true, true) => None,
            (true, false) => Some(Self::ReadWrite),
            (false, true) => Some(Self::ReadExecute),
            (false, false) => Some(Self::ReadOnly),
        }
    }

    pub const fn writable(self) -> bool {
        matches!(self, Self::ReadWrite)
    }

    pub const fn executable(self) -> bool {
        matches!(self, Self::ReadExecute)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum VmError<E> {
    Allocation(FrameAllocError),
    Backend(E),
    InvalidAddress,
    InvalidContents,
    MappingCapacity,
    TableCapacity,
    AlreadyMapped,
    NotMapped,
    Closed,
}

/// # Safety
/// Allocated tokens grant exclusive frame ownership. Return must accept this
/// allocator's tokens once their mappings and external accesses have ended.
pub unsafe trait FrameAllocator {
    fn allocate(&self) -> Result<OwnedFrame<'_>, FrameAllocError>;

    /// # Safety
    /// All previous-owner access, mappings, TLB use, and DMA use have ended.
    unsafe fn release<'pool>(
        &self,
        frame: OwnedFrame<'pool>,
    ) -> Result<(), (FrameFreeError, OwnedFrame<'pool>)>;
}

unsafe impl<const R: usize, const W: usize, const S: usize> FrameAllocator for FramePool<R, W, S> {
    fn allocate(&self) -> Result<OwnedFrame<'_>, FrameAllocError> {
        self.allocate()
    }

    unsafe fn release<'pool>(
        &self,
        frame: OwnedFrame<'pool>,
    ) -> Result<(), (FrameFreeError, OwnedFrame<'pool>)> {
        unsafe { self.release(frame) }
    }
}

/// Hardware operations for an exclusively owned, inactive user address space.
///
/// # Safety
/// Root construction must not publish a root on failure. map is transactional:
/// an error publishes no supplied frames; success returns exactly the number
/// of supplied table frames retained by the tree. All user mappings are private,
/// supervisor mappings stay inaccessible to users, and no leaf is global.
/// unmap and retire finish all required TLB work before returning.
/// Alias operations must preserve W^X across the direct map and user mappings.
/// Dropping this backend must not access or release its raw frames.
pub unsafe trait VmBackend {
    type Error;

    fn root_frame(&self) -> u64;
    fn check_inactive(&self) -> Result<(), Self::Error>;
    fn tables_needed(&self, address: u64) -> Result<usize, Self::Error>;

    /// # Safety
    /// physical is an exclusive, unpublished data frame.
    unsafe fn initialize_frame(
        &mut self,
        physical: u64,
        contents: &[u8],
    ) -> Result<(), Self::Error>;

    /// # Safety
    /// physical is owned and has no active user mapping.
    unsafe fn seal_frame(
        &mut self,
        physical: u64,
        permissions: Permissions,
    ) -> Result<(), Self::Error>;

    /// # Safety
    /// User access and required TLB invalidation have ended for physical.
    unsafe fn restore_frame(&mut self, physical: u64) -> Result<(), Self::Error>;

    /// # Safety
    /// The caller retains all supplied frames. Only returned table frames may
    /// be installed. This root is inactive and address has no existing mapping.
    unsafe fn map(
        &mut self,
        address: u64,
        physical: u64,
        permissions: Permissions,
        tables: &[u64],
    ) -> Result<usize, Self::Error>;

    /// # Safety
    /// The caller owns the mapping and root is inactive.
    unsafe fn unmap(&mut self, address: u64) -> Result<u64, Self::Error>;

    /// # Safety
    /// physical is an initialized owned data frame, with no concurrent writer.
    unsafe fn read_frame(
        &self,
        physical: u64,
        offset: usize,
        output: &mut [u8],
    ) -> Result<(), Self::Error>;

    /// # Safety
    /// No data mappings remain and root is inactive. Release raw-frame access.
    unsafe fn retire(&mut self) -> Result<(), Self::Error>;
}
