use crate::mm::frame::{
    FRAME_SIZE, FrameAllocError, FrameFreeError, FramePool, OwnedRun, PhysRange,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaLimits {
    pub max_pages: usize,
    pub physical: PhysRange,
    /// Inclusive highest byte address reachable by this device.
    pub address_limit: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaRange {
    pub address: u64,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmaState {
    Mapped,
    InFlight,
    Quiescing,
    Quiescent,
    Unmapped,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DmaError<E> {
    InvalidSize,
    InvalidLimits,
    AddressLimit,
    InvalidState,
    Allocation(FrameAllocError),
    Initialize(E),
    Map(E),
    Quiesce(E),
    Unmap(E),
    Release(FrameFreeError),
}

pub struct DmaFailure<T, E> {
    pub error: DmaError<E>,
    /// Present when cleanup did not finish. Retry retirement or retain it.
    pub retained: Option<T>,
}

impl<T, E: core::fmt::Debug> core::fmt::Debug for DmaFailure<T, E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DmaFailure")
            .field("error", &self.error)
            .field("retained", &self.retained.is_some())
            .finish()
    }
}

/// DMA translation and retirement for one exclusively owned RAM run.
///
/// # Safety
/// Initialize zeroes the entire run and establishes a retained RW+NX CPU alias;
/// failure leaves no access that prevents returning the unpublished run. Map
/// describes exactly the supplied physical bytes without starting device access;
/// failure is transactional and leaves no mapping. Address remains stable.
/// Quiesce success stops all access through this mapping, including outstanding
/// device transactions; it must not confuse masking an IRQ with stopping DMA.
/// Unmap success retires translation/IOTLB access. On either error, the mapping
/// remains owned and retryable. No method may release the supplied RAM owners.
/// Dropping an unused backend or a successfully unmapped mapping is harmless.
pub unsafe trait DmaBackend {
    type Mapping;
    type Error;

    /// # Safety
    /// The caller exclusively owns this unpublished physical RAM run.
    unsafe fn initialize(&mut self, physical: u64, bytes: u64) -> Result<(), Self::Error>;

    /// # Safety
    /// The caller retains initialized backing until explicit retirement.
    unsafe fn map(&mut self, physical: u64, bytes: u64) -> Result<Self::Mapping, Self::Error>;

    fn address(&self, mapping: &Self::Mapping) -> u64;
    fn quiesce(&mut self, mapping: &mut Self::Mapping) -> Result<(), Self::Error>;
    fn unmap(&mut self, mapping: &mut Self::Mapping) -> Result<(), Self::Error>;
}

/// Bounded bootstrap DMA storage. No user mapping or device authority is granted.
/// An abandoned buffer deliberately retains backing and translation ownership.
#[must_use = "retire DMA access before releasing its backing"]
pub struct DmaBuffer<'pool, B: DmaBackend, const R: usize, const W: usize, const S: usize> {
    pool: &'pool FramePool<R, W, S>,
    run: Option<OwnedRun<'pool>>,
    backend: Option<B>,
    mapping: Option<B::Mapping>,
    range: DmaRange,
    state: DmaState,
}

impl<'pool, B: DmaBackend, const R: usize, const W: usize, const S: usize>
    DmaBuffer<'pool, B, R, W, S>
{
    pub fn new(
        pool: &'pool FramePool<R, W, S>,
        pages: usize,
        limits: DmaLimits,
        backend: B,
    ) -> Result<Self, DmaFailure<Self, B::Error>> {
        Self::create(
            pool,
            pages,
            limits,
            backend,
            |backend, physical, bytes| unsafe { backend.initialize(physical, bytes) },
        )
    }

    /// A temporary initializer may borrow VM policy without retaining that
    /// borrow throughout device ownership. Its safety contract is initialize's.
    ///
    /// # Safety
    /// The initializer satisfies DmaBackend::initialize for the exclusive run.
    pub unsafe fn new_with_initializer(
        pool: &'pool FramePool<R, W, S>,
        pages: usize,
        limits: DmaLimits,
        backend: B,
        initialize: impl FnOnce(u64, u64) -> Result<(), B::Error>,
    ) -> Result<Self, DmaFailure<Self, B::Error>> {
        Self::create(pool, pages, limits, backend, |_, physical, bytes| {
            initialize(physical, bytes)
        })
    }

    fn create(
        pool: &'pool FramePool<R, W, S>,
        pages: usize,
        limits: DmaLimits,
        backend: B,
        initialize: impl FnOnce(&mut B, u64, u64) -> Result<(), B::Error>,
    ) -> Result<Self, DmaFailure<Self, B::Error>> {
        let bytes = u64::try_from(pages)
            .ok()
            .filter(|_| pages > 0 && pages <= limits.max_pages)
            .and_then(|pages| pages.checked_mul(FRAME_SIZE))
            .ok_or(DmaFailure {
                error: DmaError::InvalidSize,
                retained: None,
            })?;
        if limits.physical.is_empty()
            || !limits.physical.start().is_multiple_of(FRAME_SIZE)
            || !limits.physical.end().is_multiple_of(FRAME_SIZE)
        {
            return Err(DmaFailure {
                error: DmaError::InvalidLimits,
                retained: None,
            });
        }
        let run = pool
            .allocate_run_in(pages, limits.physical)
            .map_err(|error| DmaFailure {
                error: DmaError::Allocation(error),
                retained: None,
            })?;
        let mut buffer = Self {
            pool,
            range: DmaRange { address: 0, bytes },
            run: Some(run),
            backend: Some(backend),
            mapping: None,
            state: DmaState::Unmapped,
        };
        let physical = buffer.physical_address();
        // The run is exclusive and has not been exposed to CPU or device peers.
        if let Err(error) = initialize(buffer.backend.as_mut().unwrap(), physical, bytes) {
            return Err(buffer.rollback(DmaError::Initialize(error)));
        }
        // Ownership stays here through mapping failure or later cleanup errors.
        let mapping = match unsafe { buffer.backend.as_mut().unwrap().map(physical, bytes) } {
            Ok(mapping) => mapping,
            Err(error) => return Err(buffer.rollback(DmaError::Map(error))),
        };
        buffer.range.address = buffer.backend.as_ref().unwrap().address(&mapping);
        buffer.mapping = Some(mapping);
        buffer.state = DmaState::Mapped;
        if buffer
            .range
            .address
            .checked_add(bytes - 1)
            .is_none_or(|last| last > limits.address_limit)
        {
            return Err(buffer.rollback(DmaError::AddressLimit));
        }
        Ok(buffer)
    }

    pub fn physical_address(&self) -> u64 {
        self.run.as_ref().unwrap().address()
    }

    pub const fn range(&self) -> DmaRange {
        self.range
    }

    pub const fn state(&self) -> DmaState {
        self.state
    }

    /// Records possible device access before the caller programs any hardware.
    /// This neither submits work nor grants authority to program a device.
    pub fn publish(&mut self) -> Result<DmaRange, DmaError<B::Error>> {
        if self.state != DmaState::Mapped {
            return Err(DmaError::InvalidState);
        }
        self.state = DmaState::InFlight;
        Ok(self.range)
    }

    /// Stops access, removes translations, then returns backing to its pool.
    pub fn retire(mut self) -> Result<B, DmaFailure<Self, B::Error>> {
        if let Err(error) = self.cleanup() {
            return Err(DmaFailure {
                error,
                retained: Some(self),
            });
        }
        Ok(self.backend.take().unwrap())
    }

    fn rollback(mut self, error: DmaError<B::Error>) -> DmaFailure<Self, B::Error> {
        let retained = self.cleanup().is_err().then_some(self);
        DmaFailure { error, retained }
    }

    fn cleanup(&mut self) -> Result<(), DmaError<B::Error>> {
        if matches!(self.state, DmaState::Mapped | DmaState::InFlight) {
            self.state = DmaState::Quiescing;
        }
        if self.state == DmaState::Quiescing {
            self.backend
                .as_mut()
                .unwrap()
                .quiesce(self.mapping.as_mut().unwrap())
                .map_err(DmaError::Quiesce)?;
            self.state = DmaState::Quiescent;
        }
        if self.state == DmaState::Quiescent {
            self.backend
                .as_mut()
                .unwrap()
                .unmap(self.mapping.as_mut().unwrap())
                .map_err(DmaError::Unmap)?;
            self.mapping = None;
            self.state = DmaState::Unmapped;
        }
        if let Some(run) = self.run.take() {
            // No previous-owner CPU references or device translations remain.
            if let Err((error, run)) = unsafe { self.pool.release_run(run) } {
                self.run = Some(run);
                return Err(DmaError::Release(error));
            }
        }
        Ok(())
    }
}

impl<B: DmaBackend, const R: usize, const W: usize, const S: usize> Drop
    for DmaBuffer<'_, B, R, W, S>
{
    fn drop(&mut self) {
        if self.run.is_some() {
            // Drop cannot wait for devices. Leaking every owner is safer than
            // letting backend Drop tear down a still-live device translation.
            if let Some(mapping) = self.mapping.take() {
                core::mem::forget(mapping);
            }
            if let Some(backend) = self.backend.take() {
                core::mem::forget(backend);
            }
        }
    }
}
