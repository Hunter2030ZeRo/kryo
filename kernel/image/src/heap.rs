use crate::boot::memory::BootFramePool;
use core::alloc::{GlobalAlloc, Layout};
use core::mem::{align_of, size_of};
use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};
use kryo_core::mm::frame::{FRAME_SIZE, OwnedRun};
use kryo_hal::DirectMap;
use spin::{Mutex, Once};
use talc::{DefaultBinning, base::Talc, source::GlobalAllocSource};

struct Heap {
    pool: &'static BootFramePool,
    direct: DirectMap,
}

struct Header {
    owner: OwnedRun<'static>,
}

static HEAP: Once<Heap> = Once::new();
static BACKING_PAGES: AtomicU64 = AtomicU64::new(0);

// A modest block avoids the upstream source's one-MiB minimum. Runs are still
// physically contiguous; this changes object packing, not virtual backing.
const BLOCK_BYTES: usize = FRAME_SIZE as usize;

type ObjectHeap = Talc<GlobalAllocSource<PageAllocator>, DefaultBinning>;

struct AllocatorState {
    heap: ObjectHeap,
    allocations: usize,
    bytes: usize,
}

static OBJECTS: Mutex<AllocatorState> = Mutex::new(AllocatorState {
    heap: Talc::new(GlobalAllocSource::with_block_size(
        PageAllocator,
        BLOCK_BYTES,
    )),
    allocations: 0,
    bytes: 0,
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HeapStats {
    pub backing_pages: u64,
    pub allocations: usize,
    pub bytes: usize,
}

pub(crate) fn stats() -> HeapStats {
    x86_64::instructions::interrupts::without_interrupts(|| {
        let objects = OBJECTS.lock();
        HeapStats {
            backing_pages: BACKING_PAGES.load(Ordering::Relaxed),
            allocations: objects.allocations,
            bytes: objects.bytes,
        }
    })
}

#[cfg(feature = "boot-tests")]
static FAIL_NEXT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[global_allocator]
static ALLOCATOR: ObjectAllocator = ObjectAllocator;

struct ObjectAllocator;

#[derive(Debug)]
struct PageAllocator;

/// # Safety
/// The pool exclusively owns Usable RAM. Its retained HHDM must provide writable
/// supervisor aliases for each allocated run; these aliases remain NX once the
/// boot memory policy is installed. Allocation never acquires VM/object locks.
pub(crate) unsafe fn init(pool: &'static BootFramePool, direct: DirectMap) {
    assert!(HEAP.get().is_none(), "Heap already initialized");
    HEAP.call_once(|| Heap { pool, direct });
}

pub(crate) fn allocation_pages(layout: Layout) -> Option<usize> {
    let alignment = layout.align().max(align_of::<Header>());
    layout
        .size()
        .checked_add(size_of::<Header>())?
        .checked_add(alignment - 1)?
        .checked_add(FRAME_SIZE as usize - 1)
        .map(|bytes| bytes / FRAME_SIZE as usize)
}

fn admitted(layout: Layout) -> bool {
    // The pinned source adds alignment, first-heap metadata, footer/padding and
    // block rounding before constructing its backing Layout unchecked. Leave
    // two pages beyond its published first-heap metadata for those additions.
    let padding = talc::min_first_heap_layout::<DefaultBinning>()
        .size()
        .checked_add(2 * BLOCK_BYTES);
    layout.size() != 0
        && padding
            .and_then(|padding| {
                layout
                    .size()
                    .checked_add(layout.align())?
                    .checked_add(padding)
            })
            .is_some_and(|size| size <= isize::MAX as usize)
}

// IRQ exclusion starts before locking. NMI/emergency paths must never allocate.
// The nested backing path uses only the frame-pool lock and never calls back
// into the object allocator, VM or kernel-object machinery.
unsafe impl GlobalAlloc for ObjectAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        #[cfg(feature = "boot-tests")]
        if FAIL_NEXT.swap(false, core::sync::atomic::Ordering::AcqRel) {
            return ptr::null_mut();
        }
        if HEAP.get().is_none() || !admitted(layout) {
            return ptr::null_mut();
        }
        x86_64::instructions::interrupts::without_interrupts(|| {
            let mut objects = OBJECTS.lock();
            let Some(count) = objects.allocations.checked_add(1) else {
                return ptr::null_mut();
            };
            let Some(bytes) = objects.bytes.checked_add(layout.size()) else {
                return ptr::null_mut();
            };
            // The source supplies exclusive writable runs and admitted bounds
            // cover its internal size rounding. The mutex excludes all peers.
            let Some(data) = (unsafe { objects.heap.allocate(layout) }) else {
                return ptr::null_mut();
            };
            objects.allocations = count;
            objects.bytes = bytes;
            data.as_ptr()
        })
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        x86_64::instructions::interrupts::without_interrupts(|| {
            let mut objects = OBJECTS.lock();
            // GlobalAlloc requires the original live pointer and Layout.
            unsafe { objects.heap.deallocate(pointer, layout) };
            objects.allocations -= 1;
            objects.bytes -= layout.size();
        });
    }
}

// The frame pool serializes allocation across CPUs. Its paths allocate no memory;
// local IRQ exclusion also prevents an interrupted owner from being reentered.
unsafe impl GlobalAlloc for PageAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let Some(heap) = HEAP.get() else {
            return ptr::null_mut();
        };
        let Some(pages) = allocation_pages(layout) else {
            return ptr::null_mut();
        };
        x86_64::instructions::interrupts::without_interrupts(|| {
            let Ok(owner) = heap.pool.allocate_run(pages) else {
                return ptr::null_mut();
            };
            let address = owner.address();
            let allocation = (|| {
                let first = usize::try_from(heap.direct.frame_address(address).ok()?).ok()?;
                let last = address.checked_add((owner.page_count() - 1) * FRAME_SIZE)?;
                heap.direct.frame_address(last).ok()?;
                let end = first.checked_add(pages.checked_mul(FRAME_SIZE as usize)?)?;
                let alignment = layout.align().max(align_of::<Header>());
                let data = first
                    .checked_add(size_of::<Header>())?
                    .checked_add(alignment - 1)?
                    & !(alignment - 1);
                (data.checked_add(layout.size())? <= end).then_some(data)
            })();
            let Some(data) = allocation else {
                // Nothing was published, so this run has no previous accesses.
                let _ = unsafe { heap.pool.release_run(owner) };
                return ptr::null_mut();
            };
            // Header alignment is included in the data alignment. The prefix
            // lies inside the private run and is never part of the caller's slice.
            unsafe { ((data - size_of::<Header>()) as *mut Header).write(Header { owner }) };
            BACKING_PAGES.fetch_add(pages as u64, Ordering::Relaxed);
            data as *mut u8
        })
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        let Some(heap) = HEAP.get() else {
            return;
        };
        x86_64::instructions::interrupts::without_interrupts(|| {
            // GlobalAlloc requires the exact still-live allocation pointer. Read
            // moves the token before releasing the storage containing its header.
            let header = unsafe { pointer.sub(size_of::<Header>()).cast::<Header>().read() };
            let pages = header.owner.page_count();
            match unsafe { heap.pool.release_run(header.owner) } {
                Ok(()) => {
                    BACKING_PAGES.fetch_sub(pages, Ordering::Relaxed);
                }
                Err((_error, owner)) => {
                    // Preserve the owner in its unreleased prefix. No formatting
                    // or allocation is permitted on this invariant-failure path.
                    unsafe {
                        pointer
                            .sub(size_of::<Header>())
                            .cast::<Header>()
                            .write(Header { owner })
                    };
                    kryo_hal::emergency_write_str("HEAP: retained invalid release\n");
                }
            }
        });
    }
}

#[cfg(feature = "boot-tests")]
pub(crate) fn fail_next() {
    FAIL_NEXT.store(true, core::sync::atomic::Ordering::Release);
}

#[cfg(feature = "boot-tests")]
pub(crate) fn check() {
    let heap = HEAP.get().expect("Missing heap");
    let warmup = Layout::new::<u64>();
    let data = unsafe { ALLOCATOR.alloc(warmup) };
    assert!(!data.is_null());
    unsafe { ALLOCATOR.dealloc(data, warmup) };
    // Talc retains the first arena containing its metadata for kernel lifetime.
    let before = heap.pool.stats();
    let baseline = stats();
    for (size, alignment) in [(1, 1), (4096, 4096), (65_537, 65_536)] {
        let layout = Layout::from_size_align(size, alignment).unwrap();
        let data = unsafe { ALLOCATOR.alloc(layout) };
        assert!(!data.is_null(), "Heap fixture allocation failed");
        assert_eq!(data as usize % alignment, 0);
        unsafe {
            data.write_bytes(0xa5, size);
            assert_eq!(data.read(), 0xa5);
            assert_eq!(data.add(size - 1).read(), 0xa5);
            ALLOCATOR.dealloc(data, layout);
        }
        assert_eq!(heap.pool.stats(), before);
        assert_eq!(stats(), baseline);
    }
    let layout = Layout::from_size_align(64, 64).unwrap();
    let mut objects = [ptr::null_mut(); 128];
    for (index, pointer) in objects.iter_mut().enumerate() {
        *pointer = unsafe { ALLOCATOR.alloc(layout) };
        assert!(!pointer.is_null());
        unsafe { pointer.write_bytes(index as u8, layout.size()) };
    }
    let packed = stats();
    assert_eq!(packed.allocations, baseline.allocations + objects.len());
    assert_eq!(packed.bytes, baseline.bytes + objects.len() * layout.size());
    let pages = packed.backing_pages - baseline.backing_pages;
    assert!(
        pages < objects.len() as u64 / 4,
        "Small objects were not packed"
    );
    for (index, pointer) in objects.into_iter().enumerate() {
        unsafe {
            assert_eq!(pointer.read(), index as u8);
            assert_eq!(pointer.add(layout.size() - 1).read(), index as u8);
            ALLOCATOR.dealloc(pointer, layout);
        }
    }
    assert_eq!(stats(), baseline);
    assert_eq!(heap.pool.stats(), before);
    let _ = kryo_hal::print(format_args!(
        "HEAP: packed objects=128 bytes=64 backing_pages={} baseline_pages=128 retained_pages={}\n",
        pages, baseline.backing_pages
    ));
    fail_next();
    let layout = Layout::new::<u64>();
    assert!(unsafe { ALLOCATOR.alloc(layout) }.is_null());
    assert_eq!(heap.pool.stats(), before);
    assert_eq!(stats(), baseline);
    let data = unsafe { ALLOCATOR.alloc(layout) };
    assert!(!data.is_null());
    unsafe { ALLOCATOR.dealloc(data, layout) };
    assert_eq!(heap.pool.stats(), before);
    assert_eq!(stats(), baseline);
    let huge = Layout::from_size_align(isize::MAX as usize, 1).unwrap();
    assert!(unsafe { ALLOCATOR.alloc(huge) }.is_null());
    assert_eq!(stats(), baseline);
    kryo_hal::emergency_write_str("HEAP: alignment/reuse/failure OK\n");
}
