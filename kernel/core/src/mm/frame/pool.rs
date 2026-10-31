use super::{FRAME_SIZE, MemoryPlan, PhysRange};
use spin::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameInitError {
    AlreadyInitialized,
    EmptyPlan,
    InvalidPlan,
    TooManyRanges,
    BitmapTooSmall,
    SummaryTooSmall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameAllocError {
    NotInitialized,
    OutOfMemory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameFreeError {
    WrongPool,
    InvalidFrame,
    NotAllocated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameStats {
    pub total: u64,
    pub free: u64,
}

#[derive(Debug)]
struct PoolIdentity {
    _anchor: u8,
}

/// Owns one allocated frame. Dropping it leaks the allocation; release is explicit.
#[must_use = "keep the frame owner until explicit release"]
#[derive(Debug)]
pub struct OwnedFrame<'pool> {
    address: u64,
    owner: &'pool PoolIdentity,
}

impl OwnedFrame<'_> {
    pub const fn address(&self) -> u64 {
        self.address
    }
}

struct FrameState<const RANGES: usize, const WORDS: usize, const SUMMARIES: usize> {
    initialized: bool,
    ranges: [PhysRange; RANGES],
    first_frame: [u64; RANGES],
    range_count: usize,
    bitmap: [u64; WORDS],
    summary: [u64; SUMMARIES],
    summary_count: usize,
    next_summary: usize,
    stats: FrameStats,
}

/// A frame pool with owned metadata. Disable local IRQs before use; NMI use is unsupported.
pub struct FramePool<const RANGES: usize, const WORDS: usize, const SUMMARIES: usize> {
    identity: PoolIdentity,
    state: Mutex<FrameState<RANGES, WORDS, SUMMARIES>>,
}

impl<const RANGES: usize, const WORDS: usize, const SUMMARIES: usize> Default
    for FramePool<RANGES, WORDS, SUMMARIES>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<const RANGES: usize, const WORDS: usize, const SUMMARIES: usize>
    FramePool<RANGES, WORDS, SUMMARIES>
{
    pub const fn new() -> Self {
        Self {
            identity: PoolIdentity { _anchor: 0 },
            state: Mutex::new(FrameState {
                initialized: false,
                ranges: [PhysRange::EMPTY; RANGES],
                first_frame: [0; RANGES],
                range_count: 0,
                bitmap: [0; WORDS],
                summary: [0; SUMMARIES],
                summary_count: 0,
                next_summary: 0,
                stats: FrameStats { total: 0, free: 0 },
            }),
        }
    }

    /// Activates the pool once. Failure leaves its state unchanged.
    ///
    /// # Safety
    /// The plan grants exclusive allocation authority for these frames.
    /// Exclude live allocations, page tables, stacks and pool metadata.
    /// No other allocator may manage them. Boot/direct-map aliases must
    /// not be used to access them independently of the new owner.
    pub unsafe fn initialize(&self, plan: MemoryPlan<'_>) -> Result<(), FrameInitError> {
        let mut state = self.state.lock();
        if state.initialized {
            return Err(FrameInitError::AlreadyInitialized);
        }
        let ranges = plan.available();
        if ranges.len() > RANGES {
            return Err(FrameInitError::TooManyRanges);
        }

        let mut total = 0u64;
        for (index, range) in ranges.iter().enumerate() {
            if range.is_empty()
                || !range.start().is_multiple_of(FRAME_SIZE)
                || !range.end().is_multiple_of(FRAME_SIZE)
                || (index > 0 && ranges[index - 1].end() > range.start())
            {
                return Err(FrameInitError::InvalidPlan);
            }
            total = total
                .checked_add(range.len() / FRAME_SIZE)
                .ok_or(FrameInitError::InvalidPlan)?;
        }
        if total == 0 {
            return Err(FrameInitError::EmptyPlan);
        }
        if total != plan.frame_count() {
            return Err(FrameInitError::InvalidPlan);
        }

        let words =
            usize::try_from(total.div_ceil(64)).map_err(|_| FrameInitError::BitmapTooSmall)?;
        if words > WORDS {
            return Err(FrameInitError::BitmapTooSmall);
        }
        let summaries = words.div_ceil(64);
        if summaries > SUMMARIES {
            return Err(FrameInitError::SummaryTooSmall);
        }

        let mut first_frame = 0;
        for (index, range) in ranges.iter().copied().enumerate() {
            state.ranges[index] = range;
            state.first_frame[index] = first_frame;
            first_frame += range.len() / FRAME_SIZE;
        }
        state.bitmap[..words].fill(u64::MAX);
        if !total.is_multiple_of(64) {
            state.bitmap[words - 1] = (1u64 << (total % 64)) - 1;
        }
        state.summary[..summaries].fill(u64::MAX);
        if !words.is_multiple_of(64) {
            state.summary[summaries - 1] = (1u64 << (words % 64)) - 1;
        }
        state.range_count = ranges.len();
        state.summary_count = summaries;
        state.next_summary = 0;
        state.stats = FrameStats { total, free: total };
        state.initialized = true;
        Ok(())
    }

    pub fn allocate(&self) -> Result<OwnedFrame<'_>, FrameAllocError> {
        let mut state = self.state.lock();
        if !state.initialized {
            return Err(FrameAllocError::NotInitialized);
        }
        if state.stats.free == 0 {
            return Err(FrameAllocError::OutOfMemory);
        }

        let summary_index = (state.next_summary..state.summary_count)
            .chain(0..state.next_summary)
            .find(|&index| state.summary[index] != 0)
            .expect("frame summary invariant");
        let word_index =
            summary_index * 64 + state.summary[summary_index].trailing_zeros() as usize;
        let bit_index = state.bitmap[word_index].trailing_zeros();
        let frame_index = word_index as u64 * 64 + u64::from(bit_index);
        let region_index = state.first_frame[..state.range_count]
            .partition_point(|&first_frame| first_frame <= frame_index)
            - 1;
        let address = state.ranges[region_index].start()
            + (frame_index - state.first_frame[region_index]) * FRAME_SIZE;

        state.bitmap[word_index] &= !(1u64 << bit_index);
        if state.bitmap[word_index] == 0 {
            state.summary[summary_index] &= !(1u64 << (word_index % 64));
        }
        state.next_summary = summary_index;
        state.stats.free -= 1;
        Ok(OwnedFrame {
            address,
            owner: &self.identity,
        })
    }

    /// Returns the owner unchanged if the frame cannot be released.
    ///
    /// # Safety
    /// No previous-owner access may survive release. Retire owner mappings
    /// and finish applicable TLB invalidation and DMA quiescence first.
    /// Permanent kernel direct-map aliases may remain, but the previous
    /// owner must no longer use them or retain usable references.
    pub unsafe fn release<'pool>(
        &self,
        frame: OwnedFrame<'pool>,
    ) -> Result<(), (FrameFreeError, OwnedFrame<'pool>)> {
        if !core::ptr::eq(&self.identity, frame.owner) {
            return Err((FrameFreeError::WrongPool, frame));
        }
        let mut state = self.state.lock();
        let address = frame.address;
        let index =
            state.ranges[..state.range_count].partition_point(|range| range.start() <= address);
        let Some(index) = index.checked_sub(1) else {
            return Err((FrameFreeError::InvalidFrame, frame));
        };
        let range = state.ranges[index];
        if address >= range.end() || !address.is_multiple_of(FRAME_SIZE) {
            return Err((FrameFreeError::InvalidFrame, frame));
        }

        let frame_index = state.first_frame[index] + (address - range.start()) / FRAME_SIZE;
        let word_index = (frame_index / 64) as usize;
        let mask = 1u64 << (frame_index % 64);
        if state.bitmap[word_index] & mask != 0 {
            return Err((FrameFreeError::NotAllocated, frame));
        }
        state.bitmap[word_index] |= mask;
        state.summary[word_index / 64] |= 1u64 << (word_index % 64);
        state.next_summary = word_index / 64;
        state.stats.free += 1;
        Ok(())
    }

    pub fn stats(&self) -> FrameStats {
        self.state.lock().stats
    }
}
