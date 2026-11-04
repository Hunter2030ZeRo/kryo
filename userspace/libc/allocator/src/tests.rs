use super::*;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::alloc::System;
use std::sync::Arc;

#[derive(Debug, Default)]
struct Accounting {
    denied: AtomicBool,
    blocks: AtomicUsize,
    bytes: AtomicUsize,
}

#[derive(Clone, Debug, Default)]
struct Backing(Arc<Accounting>);

unsafe impl GlobalAlloc for Backing {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if self.0.denied.load(Ordering::Relaxed) {
            return ptr::null_mut();
        }
        // The pinned source acquires with alignment 1 and its Drop uses Talc's
        // chunk alignment. Normalize both so the host deallocation layout is
        // always the exact layout originally passed to System.
        let layout = Layout::from_size_align(layout.size(), layout.align().max(16)).unwrap();
        let data = unsafe { System.alloc(layout) };
        if !data.is_null() {
            self.0.blocks.fetch_add(1, Ordering::Relaxed);
            self.0.bytes.fetch_add(layout.size(), Ordering::Relaxed);
        }
        data
    }

    unsafe fn dealloc(&self, data: *mut u8, layout: Layout) {
        let layout = Layout::from_size_align(layout.size(), layout.align().max(16)).unwrap();
        unsafe { System.dealloc(data, layout) };
        self.0.blocks.fetch_sub(1, Ordering::Relaxed);
        self.0.bytes.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

#[test]
fn guard_sharing_requires_a_sync_payload() {
    fn require_send<T: Send>() {}
    fn require_sync<T: Sync>() {}

    require_send::<mutex::Guard<'_, core::cell::Cell<u8>>>();
    require_send::<mutex::Guard<'_, u64>>();
    require_sync::<mutex::Guard<'_, u64>>();

    trait AmbiguousIfSync<A> {
        fn check() {}
    }
    struct SyncMarker;
    impl<T: ?Sized> AmbiguousIfSync<()> for T {}
    impl<T: ?Sized + Sync> AmbiguousIfSync<SyncMarker> for T {}
    // A Sync implementation would make this choice ambiguous and fail to
    // compile. Cell is Send but must never be shared through a guard's Deref.
    let _ = <mutex::Guard<'_, core::cell::Cell<u8>> as AmbiguousIfSync<_>>::check;
}

#[test]
fn requested_alignment_and_size_survive_prefix() {
    let mut heap = Heap::new(Backing::default());
    for alignment in [1, 2, 8, 16, 64, 4096, 65_536] {
        for size in [1, 15, 16, 17, 4097, 65_537] {
            let data = heap.allocate(size, alignment);
            assert!(!data.is_null(), "size={size} alignment={alignment}");
            assert_eq!(data as usize % alignment, 0);
            unsafe {
                assert_eq!(read_header(data).requested, size);
                data.write_bytes(0xa5, size);
                assert_eq!(data.read(), 0xa5);
                assert_eq!(data.add(size - 1).read(), 0xa5);
                heap.deallocate(data);
            }
        }
    }
}

#[test]
fn zero_allocations_are_distinct_and_freeable() {
    let mut heap = Heap::new(Backing::default());
    let first = heap.allocate(0, 16);
    let second = heap.allocate(0, 16);
    assert!(!first.is_null());
    assert!(!second.is_null());
    assert_ne!(first, second);
    unsafe {
        assert_eq!(read_header(first).requested, 0);
        assert_eq!(read_header(second).requested, 0);
        heap.deallocate(first);
        heap.deallocate(second);
        heap.deallocate(ptr::null_mut());
    }
}

#[test]
fn invalid_and_extreme_layouts_never_reach_backing() {
    let source = Backing::default();
    let mut heap = Heap::new(source.clone());
    for (size, alignment) in [
        (1, 0),
        (1, 3),
        (usize::MAX, 1),
        (isize::MAX as usize, 1),
        (1, 1usize << (usize::BITS - 1)),
        (isize::MAX as usize / 2, 1usize << (usize::BITS - 2)),
    ] {
        assert!(heap.allocate(size, alignment).is_null());
    }
    assert_eq!(source.0.blocks.load(Ordering::Relaxed), 0);
}

#[test]
fn backing_exhaustion_is_recoverable() {
    let source = Backing::default();
    let mut heap = Heap::new(source.clone());
    source.0.denied.store(true, Ordering::Relaxed);
    assert!(heap.allocate(64, 16).is_null());
    assert_eq!(source.0.blocks.load(Ordering::Relaxed), 0);
    source.0.denied.store(false, Ordering::Relaxed);
    let data = heap.allocate(64, 16);
    assert!(!data.is_null());
    unsafe { heap.deallocate(data) };
}

#[test]
fn failed_reallocation_preserves_owner_bytes_and_alignment() {
    let source = Backing::default();
    let mut heap = Heap::new(source.clone());
    let data = heap.allocate(257, 4096);
    assert!(!data.is_null());
    unsafe { data.write_bytes(0x5a, 257) };
    source.0.denied.store(true, Ordering::Relaxed);
    assert!(unsafe { heap.reallocate(data, 1 << 20) }.is_null());
    assert!(unsafe { heap.reallocate(data, usize::MAX) }.is_null());
    unsafe {
        assert_eq!(read_header(data).requested, 257);
        assert_eq!(read_header(data).layout.align(), 4096);
        assert!((0..257).all(|offset| data.add(offset).read() == 0x5a));
    }
    source.0.denied.store(false, Ordering::Relaxed);
    let grown = unsafe { heap.reallocate(data, 1 << 20) };
    assert!(!grown.is_null());
    assert_eq!(grown as usize % 4096, 0);
    unsafe {
        assert_eq!(read_header(grown).requested, 1 << 20);
        assert!((0..257).all(|offset| grown.add(offset).read() == 0x5a));
        let shrunk = heap.reallocate(grown, 17);
        assert_eq!(shrunk, grown);
        assert_eq!(read_header(shrunk).requested, 17);
        assert_eq!(shrunk.add(16).read(), 0x5a);
        assert!(heap.reallocate(shrunk, 0).is_null());
    }
}

#[test]
fn null_reallocation_uses_malloc_alignment() {
    let mut heap = Heap::new(Backing::default());
    let data = unsafe { heap.reallocate(ptr::null_mut(), 42) };
    assert!(!data.is_null());
    assert_eq!(data as usize % MALLOC_ALIGNMENT, 0);
    unsafe {
        assert_eq!(read_header(data).requested, 42);
        heap.deallocate(data);
    }
}

#[test]
fn small_objects_pack_and_empty_secondary_arenas_return() {
    let source = Backing::default();
    {
        let mut heap = Heap::new(source.clone());
        let warmup = heap.allocate(1, 16);
        unsafe { heap.deallocate(warmup) };
        let retained = source.0.bytes.load(Ordering::Relaxed);
        let pointers: Vec<_> = (0..128).map(|_| heap.allocate(64, 16)).collect();
        assert!(pointers.iter().all(|pointer| !pointer.is_null()));
        assert!(source.0.bytes.load(Ordering::Relaxed) - retained < 16 * PAGE_BYTES);
        for data in pointers {
            unsafe { heap.deallocate(data) };
        }
        assert_eq!(source.0.bytes.load(Ordering::Relaxed), retained);
    }
    assert_eq!(source.0.blocks.load(Ordering::Relaxed), 0);
    assert_eq!(source.0.bytes.load(Ordering::Relaxed), 0);
}

#[test]
fn contended_lock_serializes_metadata_and_returns_backing() {
    let source = Backing::default();
    let heap = Arc::new(mutex::Mutex::new(Heap::new(source.clone())));
    let barrier = Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        for thread in 0..8 {
            let heap = heap.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                for round in 0..128 {
                    let size = 64 + (round * 17 + thread * 11) % 4096;
                    let alignment = 16 << (thread % 4);
                    let data = heap.lock().allocate(size, alignment);
                    assert!(!data.is_null());
                    assert_eq!(data as usize % alignment, 0);
                    let pattern = (thread * 31 + round) as u8;
                    unsafe { data.write_bytes(pattern, size) };
                    barrier.wait();
                    unsafe {
                        assert_eq!(data.read(), pattern);
                        assert_eq!(data.add(size - 1).read(), pattern);
                    }
                    barrier.wait();
                    unsafe { heap.lock().deallocate(data) };
                }
            });
        }
    });
    drop(heap);
    assert_eq!(source.0.blocks.load(Ordering::Relaxed), 0);
}

#[test]
fn lock_publishes_updates_without_allocation() {
    let count = Arc::new(mutex::Mutex::new(0usize));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let count = count.clone();
            scope.spawn(move || {
                for _ in 0..10_000 {
                    *count.lock() += 1;
                }
            });
        }
    });
    assert_eq!(*count.lock(), 80_000);
}
