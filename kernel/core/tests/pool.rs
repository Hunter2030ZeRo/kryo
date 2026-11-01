use std::collections::BTreeSet;

use kryo_core::mm::frame::{
    FRAME_SIZE, FrameAllocError, FrameFreeError, FrameInitError, FrameMetadata, FramePool,
    FrameStats, MemoryRegion, PhysRange, RegionKind, metadata_layout, normalize_memory,
};

fn usable(start: u64, pages: u64) -> MemoryRegion {
    MemoryRegion {
        range: PhysRange::from_base_len(start, pages * FRAME_SIZE).unwrap(),
        kind: RegionKind::Usable,
    }
}

fn initialize<const RANGES: usize, const WORDS: usize, const SUMMARIES: usize>(
    pool: &FramePool<RANGES, WORDS, SUMMARIES>,
    regions: &mut [MemoryRegion],
) -> Result<(), FrameInitError> {
    let mut available = [PhysRange::EMPTY; 32];
    let plan = normalize_memory(regions, &mut [], &mut available).unwrap();
    // These tests model ownership; they never access the physical addresses.
    unsafe { pool.initialize(plan) }
}

#[test]
fn initialization_is_required() {
    let pool = FramePool::<1, 1, 1>::new();
    assert!(matches!(
        pool.allocate(),
        Err(FrameAllocError::NotInitialized)
    ));
    assert_eq!(pool.stats(), FrameStats { total: 0, free: 0 });
    assert_eq!(initialize(&pool, &mut []), Err(FrameInitError::EmptyPlan));
    assert!(matches!(
        pool.allocate(),
        Err(FrameAllocError::NotInitialized)
    ));
}

#[test]
fn tail_bits_never_allocate_outside_the_plan() {
    for pages in [1, 63, 64, 65, 4095, 4096, 4097] {
        let pool = FramePool::<1, 65, 2>::new();
        initialize(&pool, &mut [usable(0, pages)]).unwrap();
        let mut owners = Vec::new();
        for page in 0..pages {
            let frame = pool.allocate().unwrap();
            assert_eq!(frame.address(), page * FRAME_SIZE);
            owners.push(frame);
        }
        assert_eq!(pool.stats().free, 0);
        assert!(matches!(pool.allocate(), Err(FrameAllocError::OutOfMemory)));
        for frame in owners {
            unsafe { pool.release(frame) }.unwrap();
        }
        assert_eq!(
            pool.stats(),
            FrameStats {
                total: pages,
                free: pages
            }
        );
    }
}

#[test]
fn released_frame_can_be_allocated_again() {
    let pool = FramePool::<1, 2, 1>::new();
    initialize(&pool, &mut [usable(0x1000, 67)]).unwrap();
    let mut owners: Vec<_> = (0..67).map(|_| pool.allocate().unwrap()).collect();
    let frame = owners.swap_remove(17);
    let address = frame.address();
    unsafe { pool.release(frame) }.unwrap();
    assert_eq!(pool.stats().free, 1);
    let replacement = pool.allocate().unwrap();
    assert_eq!(replacement.address(), address);
    owners.push(replacement);
    for frame in owners {
        unsafe { pool.release(frame) }.unwrap();
    }
    assert_eq!(pool.stats().free, 67);
}

#[test]
fn physical_holes_do_not_consume_bitmap_capacity() {
    let pool = FramePool::<2, 1, 1>::new();
    initialize(&pool, &mut [usable(0x1000, 2), usable(0x100_0000_0000, 3)]).unwrap();
    let expected = [
        0x1000,
        0x2000,
        0x100_0000_0000,
        0x100_0000_1000,
        0x100_0000_2000,
    ];
    let owners: Vec<_> = expected
        .iter()
        .map(|&address| {
            let frame = pool.allocate().unwrap();
            assert_eq!(frame.address(), address);
            frame
        })
        .collect();
    assert!(matches!(pool.allocate(), Err(FrameAllocError::OutOfMemory)));
    for frame in owners {
        unsafe { pool.release(frame) }.unwrap();
    }
}

#[test]
fn summary_crosses_and_returns_across_4096_pages() {
    let pool = FramePool::<1, 65, 2>::new();
    initialize(&pool, &mut [usable(0x1000, 4097)]).unwrap();
    let mut owners: Vec<_> = (0..4097).map(|_| pool.allocate().unwrap()).collect();
    assert!(matches!(pool.allocate(), Err(FrameAllocError::OutOfMemory)));
    let first = owners.swap_remove(0);
    let address = first.address();
    unsafe { pool.release(first) }.unwrap();
    let frame = pool.allocate().unwrap();
    assert_eq!(frame.address(), address);
    unsafe { pool.release(frame) }.unwrap();
    for frame in owners {
        unsafe { pool.release(frame) }.unwrap();
    }
    assert_eq!(pool.stats().free, 4097);
}

#[test]
fn wrong_pool_returns_the_original_owner() {
    let first = FramePool::<1, 1, 1>::new();
    let second = FramePool::<2, 2, 1>::new();
    initialize(&first, &mut [usable(0x1000, 3)]).unwrap();
    initialize(&second, &mut [usable(0x9000, 2)]).unwrap();
    let frame = first.allocate().unwrap();
    let address = frame.address();
    let (error, frame) = unsafe { second.release(frame) }.unwrap_err();
    assert_eq!(error, FrameFreeError::WrongPool);
    assert_eq!(frame.address(), address);
    assert_eq!(first.stats().free, 2);
    assert_eq!(second.stats().free, 2);
    unsafe { first.release(frame) }.unwrap();
    assert_eq!(first.stats().free, 3);
}

#[test]
fn reinitialization_preserves_live_allocations() {
    let pool = FramePool::<1, 1, 1>::new();
    initialize(&pool, &mut [usable(0x1000, 3)]).unwrap();
    let frame = pool.allocate().unwrap();
    assert_eq!(
        initialize(&pool, &mut [usable(0x9000, 2)]),
        Err(FrameInitError::AlreadyInitialized)
    );
    assert_eq!(pool.stats(), FrameStats { total: 3, free: 2 });
    unsafe { pool.release(frame) }.unwrap();
    assert_eq!(pool.stats().free, 3);
}

#[test]
fn capacity_errors_leave_the_pool_uninitialized() {
    let ranges = FramePool::<1, 4, 1>::new();
    assert_eq!(
        initialize(&ranges, &mut [usable(0x1000, 1), usable(0x9000, 1)]),
        Err(FrameInitError::TooManyRanges)
    );
    assert!(matches!(
        ranges.allocate(),
        Err(FrameAllocError::NotInitialized)
    ));
    initialize(&ranges, &mut [usable(0x1000, 1)]).unwrap();

    let bitmap = FramePool::<1, 1, 1>::new();
    assert_eq!(
        initialize(&bitmap, &mut [usable(0x1000, 65)]),
        Err(FrameInitError::BitmapTooSmall)
    );
    assert_eq!(bitmap.stats(), FrameStats { total: 0, free: 0 });
    initialize(&bitmap, &mut [usable(0x1000, 64)]).unwrap();

    let summary = FramePool::<1, 65, 1>::new();
    assert_eq!(
        initialize(&summary, &mut [usable(0x1000, 4097)]),
        Err(FrameInitError::SummaryTooSmall)
    );
    assert!(matches!(
        summary.allocate(),
        Err(FrameAllocError::NotInitialized)
    ));
    initialize(&summary, &mut [usable(0x1000, 4096)]).unwrap();
}

#[test]
fn dropping_an_owner_does_not_recycle_the_frame() {
    let pool = FramePool::<1, 1, 1>::new();
    initialize(&pool, &mut [usable(0x1000, 1)]).unwrap();
    {
        let _owner = pool.allocate().unwrap();
    }
    assert_eq!(pool.stats().free, 0);
    assert!(matches!(pool.allocate(), Err(FrameAllocError::OutOfMemory)));
}

#[test]
fn reservation_pages_are_never_allocated() {
    let pool = FramePool::<4, 2, 1>::new();
    let mut regions = [usable(0x1000, 40)];
    let mut reservations = [
        PhysRange::from_base_len(0x5101, 2).unwrap(),
        PhysRange::from_base_len(0x9000, 0x3000).unwrap(),
    ];
    let mut available = [PhysRange::EMPTY; 4];
    let plan = normalize_memory(&mut regions, &mut reservations, &mut available).unwrap();
    unsafe { pool.initialize(plan) }.unwrap();
    let mut owners = Vec::new();
    while let Ok(frame) = pool.allocate() {
        assert_ne!(frame.address(), 0x5000);
        assert!(!(0x9000..0xc000).contains(&frame.address()));
        owners.push(frame);
    }
    assert_eq!(owners.len(), 36);
    for frame in owners {
        unsafe { pool.release(frame) }.unwrap();
    }
}

#[test]
fn operations_match_an_independent_free_set() {
    let pool = FramePool::<4, 4, 1>::new();
    let mut regions = [
        usable(0x1000, 65),
        usable(0x100_0000, 103),
        usable(0x800_0000, 20),
    ];
    initialize(&pool, &mut regions).unwrap();
    let mut free: BTreeSet<_> = regions
        .iter()
        .flat_map(|region| (region.range.start()..region.range.end()).step_by(FRAME_SIZE as usize))
        .collect();
    let total = free.len() as u64;
    let mut owners = Vec::new();
    let mut random = 0x1234_5678u64;
    for _ in 0..20_000 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        if owners.is_empty() || random >> 60 < 10 {
            match pool.allocate() {
                Ok(frame) => {
                    assert!(free.remove(&frame.address()));
                    owners.push(frame);
                }
                Err(FrameAllocError::OutOfMemory) => assert!(free.is_empty()),
                Err(error) => panic!("{error:?}"),
            }
        } else {
            let frame = owners.swap_remove(random as usize % owners.len());
            let address = frame.address();
            unsafe { pool.release(frame) }.unwrap();
            assert!(free.insert(address));
        }
        assert_eq!(pool.stats().free, free.len() as u64);
    }
    for frame in owners {
        unsafe { pool.release(frame) }.unwrap();
    }
    assert_eq!(pool.stats(), FrameStats { total, free: total });
}

#[test]
fn host_threads_keep_allocations_exclusive() {
    let pool = FramePool::<1, 2, 1>::new();
    initialize(&pool, &mut [usable(0x1000, 67)]).unwrap();
    let active = std::sync::Mutex::new(BTreeSet::new());
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let pool = &pool;
            let active = &active;
            scope.spawn(move || {
                for _ in 0..1000 {
                    let frame = pool.allocate().unwrap();
                    assert!(active.lock().unwrap().insert(frame.address()));
                    std::thread::yield_now();
                    assert!(active.lock().unwrap().remove(&frame.address()));
                    unsafe { pool.release(frame) }.unwrap();
                }
            });
        }
    });
    assert!(active.lock().unwrap().is_empty());
    assert_eq!(pool.stats().free, 67);
}

#[test]
fn contiguous_runs_cross_bitmap_words_and_preserve_single_frame_summaries() {
    let pool = FramePool::<1, 3, 1>::new();
    initialize(&pool, &mut [usable(0x1000, 129)]).unwrap();
    let run = pool.allocate_run(128).unwrap();
    assert_eq!(run.address(), 0x1000);
    assert_eq!(run.page_count(), 128);
    assert_eq!(pool.stats().free, 1);
    let last = pool.allocate().unwrap();
    assert_eq!(last.address(), 0x81_000);
    assert!(matches!(
        pool.allocate_run(1),
        Err(FrameAllocError::OutOfMemory)
    ));
    unsafe { pool.release_run(run) }.unwrap();
    assert_eq!(pool.stats().free, 128);
    let frame = pool.allocate().unwrap();
    assert_eq!(frame.address(), 0x1000);
    unsafe { pool.release(frame) }.unwrap();
    unsafe { pool.release(last) }.unwrap();
    assert_eq!(pool.stats().free, 129);
}

#[test]
fn contiguous_failure_does_not_claim_frames_or_cross_reserved_holes() {
    let pool = FramePool::<2, 1, 1>::new();
    initialize(&pool, &mut [usable(0x1000, 3), usable(0x8000, 4)]).unwrap();
    let before = pool.stats();
    assert!(matches!(
        pool.allocate_run(5),
        Err(FrameAllocError::OutOfMemory)
    ));
    assert!(matches!(
        pool.allocate_run(0),
        Err(FrameAllocError::InvalidCount)
    ));
    assert!(matches!(
        pool.allocate_run(usize::MAX),
        Err(FrameAllocError::InvalidCount)
    ));
    assert_eq!(pool.stats(), before);
    let first = pool.allocate().unwrap();
    let hole = pool.allocate().unwrap();
    let third = pool.allocate().unwrap();
    unsafe { pool.release(hole) }.unwrap();
    let run = pool.allocate_run(4).unwrap();
    assert_eq!(run.address(), 0x8000);
    assert_eq!(pool.stats().free, 1);
    assert!(matches!(
        pool.allocate_run(2),
        Err(FrameAllocError::OutOfMemory)
    ));
    assert_eq!(pool.stats().free, 1);
    unsafe { pool.release_run(run) }.unwrap();
    unsafe { pool.release(first) }.unwrap();
    unsafe { pool.release(third) }.unwrap();
    assert_eq!(pool.stats(), before);
}

#[test]
fn contiguous_wrong_pool_preserves_owner_and_runs_are_explicitly_retired() {
    let pool = FramePool::<1, 1, 1>::new();
    let other = FramePool::<1, 1, 1>::new();
    assert!(matches!(
        pool.allocate_run(1),
        Err(FrameAllocError::NotInitialized)
    ));
    initialize(&pool, &mut [usable(0x1000, 6)]).unwrap();
    initialize(&other, &mut [usable(0x1000, 6)]).unwrap();
    let run = pool.allocate_run(4).unwrap();
    let (error, run) = unsafe { other.release_run(run) }.unwrap_err();
    assert_eq!(error, FrameFreeError::WrongPool);
    assert_eq!(run.address(), 0x1000);
    assert_eq!(run.page_count(), 4);
    assert_eq!(pool.stats().free, 2);
    assert_eq!(other.stats().free, 6);
    unsafe { pool.release_run(run) }.unwrap();
    let _retained = pool.allocate_run(6).unwrap();
    assert_eq!(pool.stats().free, 0);
}

#[test]
fn fragmented_free_frames_are_not_silently_treated_as_a_run() {
    let pool = FramePool::<1, 1, 1>::new();
    initialize(&pool, &mut [usable(0x1000, 8)]).unwrap();
    let frames: Vec<_> = (0..8).map(|_| pool.allocate().unwrap()).collect();
    let mut held = Vec::new();
    for (index, frame) in frames.into_iter().enumerate() {
        if index % 2 == 0 {
            unsafe { pool.release(frame) }.unwrap();
        } else {
            held.push(frame);
        }
    }
    assert_eq!(pool.stats().free, 4);
    assert!(matches!(
        pool.allocate_run(2),
        Err(FrameAllocError::OutOfMemory)
    ));
    assert_eq!(pool.stats().free, 4);
    for frame in held {
        unsafe { pool.release(frame) }.unwrap();
    }
    let run = pool.allocate_run(8).unwrap();
    unsafe { pool.release_run(run) }.unwrap();
    assert_eq!(pool.stats().free, 8);
}

fn external_metadata(frames: u64) -> FrameMetadata {
    let layout = metadata_layout(frames).unwrap();
    FrameMetadata::new(
        Box::leak(vec![0; layout.bitmap_words].into_boxed_slice()),
        Box::leak(vec![0; layout.summary_words].into_boxed_slice()),
    )
}

#[test]
fn metadata_sizing_tracks_ram_and_rejects_unrepresentable_totals() {
    let gib = 1u64 << 30;
    let layout = metadata_layout(128 * gib / FRAME_SIZE).unwrap();
    assert_eq!(layout.bitmap_words, 524_288);
    assert_eq!(layout.summary_words, 8_192);
    assert_eq!(layout.bytes, 4 * 1024 * 1024 + 64 * 1024);
    assert_eq!(metadata_layout(0), Err(FrameInitError::EmptyPlan));
    assert_eq!(metadata_layout(u64::MAX), Err(FrameInitError::InvalidPlan));
    for frames in [1, 63, 64, 65, 4095, 4096, 4097] {
        let layout = metadata_layout(frames).unwrap();
        assert_eq!(layout.bitmap_words as u64, frames.div_ceil(64));
        assert_eq!(layout.summary_words, layout.bitmap_words.div_ceil(64));
        assert_eq!(
            layout.bytes,
            (layout.bitmap_words + layout.summary_words) * 8
        );
    }
}

#[test]
fn external_metadata_manages_128_gib_without_backing_ram() {
    let bytes = 128u64 << 30;
    let pages = bytes / FRAME_SIZE;
    let pool = FramePool::<1, 0, 0>::new();
    let mut regions = [usable(0, pages)];
    let mut available = [PhysRange::EMPTY; 1];
    let plan = normalize_memory(&mut regions, &mut [], &mut available).unwrap();
    // Only bitmap metadata is backed on the host; no payload RAM is accessed.
    unsafe { pool.initialize_external(plan, external_metadata(pages)) }.unwrap();
    assert_eq!(
        pool.stats(),
        FrameStats {
            total: pages,
            free: pages
        }
    );
    let around_four_gib =
        PhysRange::from_base_len((4u64 << 30) - FRAME_SIZE, FRAME_SIZE * 2).unwrap();
    let low = pool.allocate_in(around_four_gib).unwrap();
    let high = pool.allocate_in(around_four_gib).unwrap();
    assert_eq!(low.address(), (4u64 << 30) - FRAME_SIZE);
    assert_eq!(high.address(), 4u64 << 30);
    assert!(matches!(
        pool.allocate_in(around_four_gib),
        Err(FrameAllocError::OutOfMemory)
    ));
    let tail_range = PhysRange::from_base_len(bytes - FRAME_SIZE, FRAME_SIZE).unwrap();
    let tail = pool.allocate_in(tail_range).unwrap();
    assert_eq!(tail.address(), bytes - FRAME_SIZE);
    assert!(matches!(
        pool.allocate_in(PhysRange::from_base_len(bytes, FRAME_SIZE).unwrap()),
        Err(FrameAllocError::OutOfMemory)
    ));
    assert_eq!(pool.stats().free, pages - 3);
    for frame in [low, high, tail] {
        unsafe { pool.release(frame) }.unwrap();
    }
    assert_eq!(pool.stats().free, pages);
    let tail = pool.allocate_in(tail_range).unwrap();
    unsafe { pool.release(tail) }.unwrap();
}

#[test]
fn external_metadata_extends_past_128_gib_and_sparse_high_addresses() {
    let pages = (128u64 << 30) / FRAME_SIZE + 1;
    let start = 1u64 << 40;
    let pool = FramePool::<2, 0, 0>::new();
    let mut regions = [usable(0x1000, 1), usable(start, pages - 1)];
    let mut available = [PhysRange::EMPTY; 2];
    let plan = normalize_memory(&mut regions, &mut [], &mut available).unwrap();
    unsafe { pool.initialize_external(plan, external_metadata(pages)) }.unwrap();
    let target = start + (pages - 2) * FRAME_SIZE;
    let frame = pool
        .allocate_in(PhysRange::from_base_len(target, FRAME_SIZE).unwrap())
        .unwrap();
    assert_eq!(frame.address(), target);
    unsafe { pool.release(frame) }.unwrap();
    assert_eq!(pool.stats().free, pages);
}

#[test]
fn external_initialization_failure_returns_unchanged_storage_for_retry() {
    let pool = FramePool::<1, 0, 0>::new();
    let mut regions = [usable(0, 65)];
    let mut available = [PhysRange::EMPTY; 1];
    let plan = normalize_memory(&mut regions, &mut [], &mut available).unwrap();
    let bitmap = Box::leak(vec![0x1234; 1].into_boxed_slice());
    let summary = Box::leak(vec![0x5678; 1].into_boxed_slice());
    let failure =
        unsafe { pool.initialize_external(plan, FrameMetadata::new(bitmap, summary)) }.unwrap_err();
    assert_eq!(failure.error, FrameInitError::BitmapTooSmall);
    assert_eq!(pool.stats(), FrameStats { total: 0, free: 0 });
    let (bitmap, summary) = failure.metadata.into_parts();
    assert_eq!(bitmap, &[0x1234]);
    assert_eq!(summary, &[0x5678]);
    let mut regions = [usable(0x8000, 64)];
    let plan = normalize_memory(&mut regions, &mut [], &mut available).unwrap();
    unsafe { pool.initialize_external(plan, FrameMetadata::new(bitmap, summary)) }.unwrap();
    let frame = pool.allocate().unwrap();
    assert_eq!(frame.address(), 0x8000);
    unsafe { pool.release(frame) }.unwrap();
}

#[test]
fn external_summary_failure_and_double_init_do_not_publish_or_reset() {
    let pool = FramePool::<1, 0, 0>::new();
    let mut regions = [usable(0x1000, 4097)];
    let mut available = [PhysRange::EMPTY; 1];
    let plan = normalize_memory(&mut regions, &mut [], &mut available).unwrap();
    let bitmap = Box::leak(vec![0x1234; 65].into_boxed_slice());
    let summary = Box::leak(vec![0x5678; 1].into_boxed_slice());
    let failure =
        unsafe { pool.initialize_external(plan, FrameMetadata::new(bitmap, summary)) }.unwrap_err();
    assert_eq!(failure.error, FrameInitError::SummaryTooSmall);
    let (bitmap, summary) = failure.metadata.into_parts();
    assert!(bitmap.iter().all(|word| *word == 0x1234));
    assert_eq!(summary, &[0x5678]);
    let plan = normalize_memory(&mut regions, &mut [], &mut available).unwrap();
    unsafe { pool.initialize_external(plan, external_metadata(4097)) }.unwrap();
    let frame = pool.allocate().unwrap();
    let before = pool.stats();
    let plan = normalize_memory(&mut regions, &mut [], &mut available).unwrap();
    let failure =
        unsafe { pool.initialize_external(plan, FrameMetadata::new(bitmap, summary)) }.unwrap_err();
    assert_eq!(failure.error, FrameInitError::AlreadyInitialized);
    assert_eq!(pool.stats(), before);
    let (bitmap, summary) = failure.metadata.into_parts();
    assert!(bitmap.iter().all(|word| *word == 0x1234));
    assert_eq!(summary, &[0x5678]);
    unsafe { pool.release(frame) }.unwrap();
}

#[test]
fn selected_allocations_respect_holes_bounds_and_existing_owners() {
    let pool = FramePool::<4, 2, 1>::new();
    assert!(matches!(
        pool.allocate_in(PhysRange::EMPTY),
        Err(FrameAllocError::NotInitialized)
    ));
    initialize(&pool, &mut [usable(0x1000, 65), usable(0x80_000, 2)]).unwrap();
    let before = pool.stats();
    for range in [
        PhysRange::EMPTY,
        PhysRange::from_base_len(0x1001, 4096).unwrap(),
    ] {
        assert!(matches!(
            pool.allocate_in(range),
            Err(FrameAllocError::InvalidRange)
        ));
        assert_eq!(pool.stats(), before);
    }
    let span = PhysRange::from_base_len(0x1000 + 63 * FRAME_SIZE, FRAME_SIZE * 3).unwrap();
    let first = pool.allocate_in(span).unwrap();
    let second = pool.allocate_in(span).unwrap();
    assert_eq!(first.address(), 0x40_000);
    assert_eq!(second.address(), 0x41_000);
    assert!(matches!(
        pool.allocate_in(span),
        Err(FrameAllocError::OutOfMemory)
    ));
    let hole = PhysRange::from_base_len(0x42_000, 0x80_000 - 0x42_000).unwrap();
    assert!(matches!(
        pool.allocate_in(hole),
        Err(FrameAllocError::OutOfMemory)
    ));
    assert_eq!(pool.stats().free, before.free - 2);
    unsafe { pool.release(first) }.unwrap();
    unsafe { pool.release(second) }.unwrap();
    let run = pool.allocate_run(65).unwrap();
    assert_eq!(run.address(), 0x1000);
    assert!(matches!(
        pool.allocate_in(span),
        Err(FrameAllocError::OutOfMemory)
    ));
    unsafe { pool.release_run(run) }.unwrap();
    assert_eq!(pool.stats(), before);
}

#[test]
fn boot_reserved_external_metadata_remains_outside_pool_authority() {
    let pool = FramePool::<2, 0, 0>::new();
    let mut regions = [usable(0x1000, 130)];
    let metadata_pages = PhysRange::from_base_len(0x4000, FRAME_SIZE * 2).unwrap();
    let mut reservations = [metadata_pages];
    let mut available = [PhysRange::EMPTY; 2];
    let plan = normalize_memory(&mut regions, &mut reservations, &mut available).unwrap();
    unsafe { pool.initialize_external(plan, external_metadata(130)) }.unwrap();
    assert_eq!(pool.stats().total, 128);
    assert!(matches!(
        pool.allocate_in(metadata_pages),
        Err(FrameAllocError::OutOfMemory)
    ));
    let mut frames = Vec::new();
    while let Ok(frame) = pool.allocate() {
        assert!(!(0x4000..0x6000).contains(&frame.address()));
        frames.push(frame);
    }
    assert_eq!(frames.len(), 128);
    for frame in frames {
        unsafe { pool.release(frame) }.unwrap();
    }
    assert_eq!(pool.stats().free, 128);
}
