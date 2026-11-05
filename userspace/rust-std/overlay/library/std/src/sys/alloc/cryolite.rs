use super::MIN_ALIGN;
use crate::alloc::Layout;
use crate::cmp;
use crate::ptr;

pub unsafe fn alloc(layout: Layout) -> *mut u8 {
    if layout.align() <= MIN_ALIGN {
        unsafe { libc::malloc(layout.size()).cast() }
    } else {
        let Some(size) = layout.size().checked_add(layout.align() - 1) else {
            return ptr::null_mut();
        };
        let size = size & !(layout.align() - 1);
        unsafe { libc::aligned_alloc(layout.align(), size).cast() }
    }
}

pub unsafe fn alloc_zeroed(layout: Layout) -> *mut u8 {
    if layout.align() <= MIN_ALIGN {
        unsafe { libc::calloc(1, layout.size()).cast() }
    } else {
        let pointer = unsafe { alloc(layout) };
        if !pointer.is_null() {
            unsafe { pointer.write_bytes(0, layout.size()) };
        }
        pointer
    }
}

pub unsafe fn dealloc(pointer: *mut u8, _layout: Layout) {
    unsafe { libc::free(pointer.cast()) };
}

pub unsafe fn realloc(pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
    if layout.align() <= MIN_ALIGN {
        unsafe { libc::realloc(pointer.cast(), size).cast() }
    } else {
        let Ok(new_layout) = Layout::from_size_align(size, layout.align()) else {
            return ptr::null_mut();
        };
        let replacement = unsafe { alloc(new_layout) };
        if !replacement.is_null() {
            unsafe {
                ptr::copy_nonoverlapping(pointer, replacement, cmp::min(layout.size(), size));
                dealloc(pointer, layout);
            }
        }
        replacement
    }
}
