use super::{FRAME_SIZE, MemoryPlan, PhysRange};
use core::ops::{Deref, DerefMut};
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
pub struct FrameMetadataLayout {
    pub bitmap_words: usize,
    pub summary_words: usize,
    pub bytes: usize,
}

/// Sizes dense allocation metadata; physical holes consume no bitmap bits.
pub fn metadata_layout(frames: u64) -> Result<FrameMetadataLayout, FrameInitError> {
    if frames == 0 {
        return Err(FrameInitError::EmptyPlan);
    }
    frames
        .checked_mul(FRAME_SIZE)
        .ok_or(FrameInitError::InvalidPlan)?;
    let bitmap_words =
        usize::try_from(frames.div_ceil(64)).map_err(|_| FrameInitError::BitmapTooSmall)?;
    let summary_words = bitmap_words.div_ceil(64);
    let bytes = bitmap_words
        .checked_add(summary_words)
        .and_then(|words| words.checked_mul(size_of::<u64>()))
        .ok_or(FrameInitError::InvalidPlan)?;
    Ok(FrameMetadataLayout {
        bitmap_words,
        summary_words,
        bytes,
    })
}

/// Exclusive metadata backing retained for the lifetime of an initialized pool.
pub struct FrameMetadata {
    bitmap: &'static mut [u64],
    summary: &'static mut [u64],
}

impl FrameMetadata {
    pub fn new(bitmap: &'static mut [u64], summary: &'static mut [u64]) -> Self {
        Self { bitmap, summary }
    }

    pub fn into_parts(self) -> (&'static mut [u64], &'static mut [u64]) {
        (self.bitmap, self.summary)
    }
}

impl core::fmt::Debug for FrameMetadata {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("FrameMetadata")
            .field("bitmap_words", &self.bitmap.len())
            .field("summary_words", &self.summary.len())
            .finish()
    }
}

#[derive(Debug)]
pub struct FrameMetadataError {
    pub error: FrameInitError,
    pub metadata: FrameMetadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameAllocError {
    NotInitialized,
    OutOfMemory,
    InvalidCount,
    InvalidRange,
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

/// Owns one physically contiguous allocation. Release remains explicit.
#[must_use = "keep the run owner until explicit release"]
#[derive(Debug)]
pub struct OwnedRun<'pool> {
    address: u64,
    pages: u64,
    owner: &'pool PoolIdentity,
}

impl OwnedRun<'_> {
    pub const fn address(&self) -> u64 {
        self.address
    }

    pub const fn page_count(&self) -> u64 {
        self.pages
    }
}

enum Words<const INLINE: usize> {
    Inline([u64; INLINE]),
    External(&'static mut [u64]),
}

impl<const INLINE: usize> Deref for Words<INLINE> {
    type Target = [u64];
    fn deref(&self) -> &[u64] {
        match self {
            Self::Inline(words) => words,
            Self::External(words) => words,
        }
    }
}

impl<const INLINE: usize> DerefMut for Words<INLINE> {
    fn deref_mut(&mut self) -> &mut [u64] {
        match self {
            Self::Inline(words) => words,
            Self::External(words) => words,
        }
    }
}

struct FrameState<const RANGES: usize, const WORDS: usize, const SUMMARIES: usize> {
    initialized: bool,
    ranges: [PhysRange; RANGES],
    first_frame: [u64; RANGES],
    range_count: usize,
    bitmap: Words<WORDS>,
    summary: Words<SUMMARIES>,
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
                bitmap: Words::Inline([0; WORDS]),
                summary: Words::Inline([0; SUMMARIES]),
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
        let layout = state.validate(&plan, state.bitmap.len(), state.summary.len())?;
        state.activate(plan.available(), plan.frame_count(), layout);
        Ok(())
    }

    /// Activates a pool using boot-reserved backing, without heap allocation.
    /// Failure returns both buffers untouched and leaves the pool unchanged.
    ///
    /// # Safety
    /// The allocation-authority requirements of initialize apply. Keep the
    /// supplied buffers mapped, exclusively writable, and outside this and every
    /// other frame pool for the full pool lifetime. No aliases or device access
    /// may read or mutate their bytes while the pool owns these borrows.
    pub unsafe fn initialize_external(
        &self,
        plan: MemoryPlan<'_>,
        metadata: FrameMetadata,
    ) -> Result<(), FrameMetadataError> {
        let mut state = self.state.lock();
        let layout = match state.validate(&plan, metadata.bitmap.len(), metadata.summary.len()) {
            Ok(layout) => layout,
            Err(error) => return Err(FrameMetadataError { error, metadata }),
        };
        state.bitmap = Words::External(metadata.bitmap);
        state.summary = Words::External(metadata.summary);
        state.activate(plan.available(), plan.frame_count(), layout);
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
        let address = state.claim(frame_index);
        Ok(OwnedFrame {
            address,
            owner: &self.identity,
        })
    }

    /// Allocates one free frame wholly inside the requested physical span.
    /// The range selects from this pool's authority; it grants no new authority.
    pub fn allocate_in(&self, range: PhysRange) -> Result<OwnedFrame<'_>, FrameAllocError> {
        let mut state = self.state.lock();
        if !state.initialized {
            return Err(FrameAllocError::NotInitialized);
        }
        if range.is_empty()
            || !range.start().is_multiple_of(FRAME_SIZE)
            || !range.end().is_multiple_of(FRAME_SIZE)
        {
            return Err(FrameAllocError::InvalidRange);
        }
        for region in 0..state.range_count {
            let managed = state.ranges[region];
            let start = range.start().max(managed.start());
            let end = range.end().min(managed.end());
            if start >= end {
                continue;
            }
            let first = state.first_frame[region] + (start - managed.start()) / FRAME_SIZE;
            let end = state.first_frame[region] + (end - managed.start()) / FRAME_SIZE;
            for word in (first / 64) as usize..=((end - 1) / 64) as usize {
                let mut bits = state.bitmap[word];
                if word == (first / 64) as usize {
                    bits &= u64::MAX << (first % 64);
                }
                if word == ((end - 1) / 64) as usize && !end.is_multiple_of(64) {
                    bits &= (1 << (end % 64)) - 1;
                }
                if bits != 0 {
                    let index = word as u64 * 64 + u64::from(bits.trailing_zeros());
                    let address = state.claim(index);
                    return Ok(OwnedFrame {
                        address,
                        owner: &self.identity,
                    });
                }
            }
        }
        Err(FrameAllocError::OutOfMemory)
    }

    /// Allocates a contiguous run without modifying the pool on failure.
    /// Runs never cross a hole between physical memory ranges.
    pub fn allocate_run(&self, pages: usize) -> Result<OwnedRun<'_>, FrameAllocError> {
        self.allocate_run_in(
            pages,
            PhysRange::from_base_len(0, !(FRAME_SIZE - 1)).unwrap(),
        )
    }

    /// Allocates a contiguous run wholly inside a page-aligned physical span.
    /// The range restricts existing pool authority, including across RAM holes.
    pub fn allocate_run_in(
        &self,
        pages: usize,
        range: PhysRange,
    ) -> Result<OwnedRun<'_>, FrameAllocError> {
        let mut state = self.state.lock();
        if !state.initialized {
            return Err(FrameAllocError::NotInitialized);
        }
        let pages = u64::try_from(pages).map_err(|_| FrameAllocError::InvalidCount)?;
        if pages == 0 || pages.checked_mul(FRAME_SIZE).is_none() {
            return Err(FrameAllocError::InvalidCount);
        }
        if range.is_empty()
            || !range.start().is_multiple_of(FRAME_SIZE)
            || !range.end().is_multiple_of(FRAME_SIZE)
        {
            return Err(FrameAllocError::InvalidRange);
        }
        if pages > state.stats.free {
            return Err(FrameAllocError::OutOfMemory);
        }

        let mut found = None;
        for region in 0..state.range_count {
            let managed = state.ranges[region];
            let start = range.start().max(managed.start());
            let end = range.end().min(managed.end());
            if start >= end {
                continue;
            }
            let first = state.first_frame[region] + (start - managed.start()) / FRAME_SIZE;
            let count = (end - start) / FRAME_SIZE;
            if count < pages {
                continue;
            }
            let mut length = 0;
            for offset in 0..count {
                let index = first + offset;
                let free = state.bitmap[(index / 64) as usize] & (1 << (index % 64)) != 0;
                length = if free { length + 1 } else { 0 };
                if length == pages {
                    let start = index + 1 - pages;
                    let address =
                        managed.start() + (start - state.first_frame[region]) * FRAME_SIZE;
                    found = Some((start, address));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let (first, address) = found.ok_or(FrameAllocError::OutOfMemory)?;
        for index in first..first + pages {
            let word = (index / 64) as usize;
            state.bitmap[word] &= !(1 << (index % 64));
            if state.bitmap[word] == 0 {
                state.summary[word / 64] &= !(1 << (word % 64));
            }
        }
        state.stats.free -= pages;
        state.next_summary = (first / 4096) as usize;
        Ok(OwnedRun {
            address,
            pages,
            owner: &self.identity,
        })
    }

    /// Returns a run atomically, preserving its owner if validation fails.
    ///
    /// # Safety
    /// No previous-owner access may survive release. Complete mapping, TLB and
    /// device retirement for every frame, as required by `release`.
    pub unsafe fn release_run<'pool>(
        &self,
        run: OwnedRun<'pool>,
    ) -> Result<(), (FrameFreeError, OwnedRun<'pool>)> {
        if !core::ptr::eq(&self.identity, run.owner) {
            return Err((FrameFreeError::WrongPool, run));
        }
        let mut state = self.state.lock();
        let candidate =
            state.ranges[..state.range_count].partition_point(|range| range.start() <= run.address);
        let Some(region) = candidate.checked_sub(1) else {
            return Err((FrameFreeError::InvalidFrame, run));
        };
        let range = state.ranges[region];
        let end = run
            .pages
            .checked_mul(FRAME_SIZE)
            .and_then(|size| run.address.checked_add(size));
        if run.pages == 0
            || !run.address.is_multiple_of(FRAME_SIZE)
            || !end.is_some_and(|end| end <= range.end())
        {
            return Err((FrameFreeError::InvalidFrame, run));
        }
        let first = state.first_frame[region] + (run.address - range.start()) / FRAME_SIZE;
        for index in first..first + run.pages {
            if state.bitmap[(index / 64) as usize] & (1 << (index % 64)) != 0 {
                return Err((FrameFreeError::NotAllocated, run));
            }
        }
        for index in first..first + run.pages {
            let word = (index / 64) as usize;
            state.bitmap[word] |= 1 << (index % 64);
            state.summary[word / 64] |= 1 << (word % 64);
        }
        state.next_summary = (first / 4096) as usize;
        state.stats.free += run.pages;
        Ok(())
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

impl<const RANGES: usize, const WORDS: usize, const SUMMARIES: usize>
    FrameState<RANGES, WORDS, SUMMARIES>
{
    fn validate(
        &self,
        plan: &MemoryPlan<'_>,
        bitmap_words: usize,
        summary_words: usize,
    ) -> Result<FrameMetadataLayout, FrameInitError> {
        if self.initialized {
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
        if total != plan.frame_count() {
            return Err(FrameInitError::InvalidPlan);
        }
        let layout = metadata_layout(total)?;
        if layout.bitmap_words > bitmap_words {
            return Err(FrameInitError::BitmapTooSmall);
        }
        if layout.summary_words > summary_words {
            return Err(FrameInitError::SummaryTooSmall);
        }
        Ok(layout)
    }

    fn activate(&mut self, ranges: &[PhysRange], total: u64, layout: FrameMetadataLayout) {
        let mut first_frame = 0;
        for (index, range) in ranges.iter().copied().enumerate() {
            self.ranges[index] = range;
            self.first_frame[index] = first_frame;
            first_frame += range.len() / FRAME_SIZE;
        }
        self.bitmap[..layout.bitmap_words].fill(u64::MAX);
        if !total.is_multiple_of(64) {
            self.bitmap[layout.bitmap_words - 1] = (1 << (total % 64)) - 1;
        }
        self.summary[..layout.summary_words].fill(u64::MAX);
        if !layout.bitmap_words.is_multiple_of(64) {
            self.summary[layout.summary_words - 1] = (1 << (layout.bitmap_words % 64)) - 1;
        }
        self.range_count = ranges.len();
        self.summary_count = layout.summary_words;
        self.next_summary = 0;
        self.stats = FrameStats { total, free: total };
        self.initialized = true;
    }

    fn claim(&mut self, frame_index: u64) -> u64 {
        let region =
            self.first_frame[..self.range_count].partition_point(|&first| first <= frame_index) - 1;
        let address =
            self.ranges[region].start() + (frame_index - self.first_frame[region]) * FRAME_SIZE;
        let word = (frame_index / 64) as usize;
        self.bitmap[word] &= !(1 << (frame_index % 64));
        if self.bitmap[word] == 0 {
            self.summary[word / 64] &= !(1 << (word % 64));
        }
        self.next_summary = word / 64;
        self.stats.free -= 1;
        address
    }
}
