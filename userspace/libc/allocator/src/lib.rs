#![cfg_attr(not(test), no_std)]

use core::alloc::{GlobalAlloc, Layout};
use core::mem::{align_of, size_of};
use core::ptr;
use talc::{DefaultBinning, base::Talc, source::GlobalAllocSource};

mod mutex;
#[cfg(not(test))]
mod native;

const PAGE_BYTES: usize = 4096;
const MALLOC_ALIGNMENT: usize = 16;

#[derive(Clone, Copy)]
struct Header {
    layout: Layout,
    offset: usize,
    requested: usize,
}

struct Heap<G: GlobalAlloc + core::fmt::Debug> {
    objects: Talc<GlobalAllocSource<G>, DefaultBinning>,
}

impl<G: GlobalAlloc + core::fmt::Debug> Heap<G> {
    const fn new(backing: G) -> Self {
        Self {
            objects: Talc::new(GlobalAllocSource::with_block_size(backing, PAGE_BYTES)),
        }
    }

    fn allocate(&mut self, size: usize, alignment: usize) -> *mut u8 {
        let Some((layout, offset)) = allocation_layout(size, alignment) else {
            return ptr::null_mut();
        };
        // The checked layout is nonempty and leaves room for the pinned source's
        // unchecked alignment, first-arena metadata and block rounding.
        let Some(base) = (unsafe { self.objects.allocate(layout) }) else {
            return ptr::null_mut();
        };
        // The prefix and payload lie in one exclusive allocation. offset and
        // base alignment also align the private Header immediately before it.
        unsafe {
            let data = base.as_ptr().add(offset);
            data.sub(size_of::<Header>())
                .cast::<Header>()
                .write(Header {
                    layout,
                    offset,
                    requested: size,
                });
            data
        }
    }

    unsafe fn deallocate(&mut self, data: *mut u8) {
        if data.is_null() {
            return;
        }
        // The caller supplies the original live allocation, never a foreign,
        // interior or already-freed pointer, as required by C free().
        let header = unsafe { read_header(data) };
        unsafe {
            self.objects
                .deallocate(data.sub(header.offset), header.layout)
        };
    }

    unsafe fn reallocate(&mut self, data: *mut u8, size: usize) -> *mut u8 {
        if data.is_null() {
            return self.allocate(size, MALLOC_ALIGNMENT);
        }
        if size == 0 {
            unsafe { self.deallocate(data) };
            return ptr::null_mut();
        }
        let header = unsafe { read_header(data) };
        let Some((layout, offset)) = allocation_layout(size, header.layout.align()) else {
            return ptr::null_mut();
        };
        debug_assert_eq!(offset, header.offset);
        let base = unsafe { data.sub(header.offset) };
        // The old layout remains valid on growth failure; Talc shrink is
        // infallible and cannot remove the unchanged prefix.
        if unsafe {
            self.objects
                .try_realloc_in_place(base, header.layout, layout.size())
        } {
            unsafe {
                data.sub(size_of::<Header>())
                    .cast::<Header>()
                    .write(Header {
                        layout,
                        offset,
                        requested: size,
                    })
            };
            return data;
        }
        let replacement = self.allocate(size, header.layout.align());
        if replacement.is_null() {
            return replacement;
        }
        // Both allocations are still live and disjoint; failed allocation above
        // left every byte and the ownership of the original untouched.
        unsafe {
            ptr::copy_nonoverlapping(data, replacement, header.requested.min(size));
            self.deallocate(data);
        }
        replacement
    }
}

fn allocation_layout(size: usize, alignment: usize) -> Option<(Layout, usize)> {
    if !alignment.is_power_of_two() {
        return None;
    }
    let alignment = alignment.max(align_of::<Header>());
    let offset = size_of::<Header>().checked_add(alignment - 1)? & !(alignment - 1);
    let total = offset.checked_add(size.max(1))?;
    let layout = Layout::from_size_align(total, alignment).ok()?;
    let padding = talc::min_first_heap_layout::<DefaultBinning>()
        .size()
        .checked_add(2 * PAGE_BYTES)?;
    let backing = total.checked_add(alignment)?.checked_add(padding)?;
    (backing <= isize::MAX as usize).then_some((layout, offset))
}

unsafe fn read_header(data: *mut u8) -> Header {
    unsafe { data.sub(size_of::<Header>()).cast::<Header>().read() }
}

#[cfg(not(test))]
static HEAP: mutex::Mutex<Heap<native::PageAllocator>> =
    mutex::Mutex::new(Heap::new(native::PageAllocator));

#[cfg(not(test))]
#[global_allocator]
static RUST_ALLOCATOR: RustAllocator = RustAllocator;

#[cfg(not(test))]
struct RustAllocator;

// Talc's allocator API dependency links alloc. Keep any resulting Rust
// allocation on this same fallible heap instead of requiring a second runtime.
#[cfg(not(test))]
unsafe impl GlobalAlloc for RustAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        __cryolite_allocate(layout.size(), layout.align())
    }

    unsafe fn dealloc(&self, data: *mut u8, _layout: Layout) {
        unsafe { __cryolite_deallocate(data) };
    }
}

/// Allocates size bytes, or a unique freeable minimum block when size is zero.
/// Invalid alignment, layout overflow and backing exhaustion return null.
#[cfg(not(test))]
#[unsafe(no_mangle)]
pub extern "C" fn __cryolite_allocate(size: usize, alignment: usize) -> *mut u8 {
    HEAP.lock().allocate(size, alignment)
}

/// # Safety
/// The pointer is null or an original live pointer returned by this bridge.
#[cfg(not(test))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __cryolite_deallocate(data: *mut u8) {
    unsafe { HEAP.lock().deallocate(data) };
}

/// # Safety
/// The pointer is null or an original live pointer exclusively owned by the
/// caller. A null failure preserves it. A zero size frees it and returns null.
#[cfg(not(test))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __cryolite_reallocate(data: *mut u8, size: usize) -> *mut u8 {
    unsafe { HEAP.lock().reallocate(data, size) }
}

/// # Safety
/// The pointer is null or a live allocation that cannot be freed or resized
/// during this call. Returns the requested size, not allocator slack.
#[cfg(not(test))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __cryolite_allocation_size(data: *mut u8) -> usize {
    if data.is_null() {
        return 0;
    }
    unsafe { read_header(data).requested }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    native::abort()
}

#[cfg(test)]
mod tests;
