use std::collections::BTreeSet;

use kryo_core::mm::frame::{
    FRAME_SIZE, FrameAllocError, FrameFreeError, FrameInitError, FramePool, FrameStats,
    MemoryRegion, PhysRange, RegionKind, normalize_memory,
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
