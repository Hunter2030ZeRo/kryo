use crate::boot::memory::BootFramePool;
use core::alloc::{GlobalAlloc, Layout};
use core::mem::{align_of, size_of};
use core::ptr;
use kryo_core::mm::frame::{FRAME_SIZE, OwnedRun};
use kryo_hal::DirectMap;
use spin::Once;

struct Heap {
    pool: &'static BootFramePool,
    direct: DirectMap,
}

struct Header {
    owner: OwnedRun<'static>,
}

static HEAP: Once<Heap> = Once::new();

#[cfg(feature = "boot-tests")]
static FAIL_NEXT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[global_allocator]
static ALLOCATOR: PageAllocator = PageAllocator;

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

// The frame pool serializes allocation across CPUs. Its paths allocate no memory;
// local IRQ exclusion also prevents an interrupted owner from being reentered.
unsafe impl GlobalAlloc for PageAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        #[cfg(feature = "boot-tests")]
        if FAIL_NEXT.swap(false, core::sync::atomic::Ordering::AcqRel) {
            return ptr::null_mut();
        }
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
            if let Err((_error, _owner)) = unsafe { heap.pool.release_run(header.owner) } {
                // A rejected release leaves the run allocated. Never recycle
                // frames after an invariant failure. No formatting here.
                kryo_hal::emergency_write_str("HEAP: retained invalid release\n");
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
    let before = heap.pool.stats();
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
    }
    fail_next();
    let layout = Layout::new::<u64>();
    assert!(unsafe { ALLOCATOR.alloc(layout) }.is_null());
    assert_eq!(heap.pool.stats(), before);
    let data = unsafe { ALLOCATOR.alloc(layout) };
    assert!(!data.is_null());
    unsafe { ALLOCATOR.dealloc(data, layout) };
    assert_eq!(heap.pool.stats(), before);
    kryo_hal::emergency_write_str("HEAP: alignment/reuse/failure OK\n");
}
