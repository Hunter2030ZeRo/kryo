use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use crate::PAGE_BYTES;

const VM_RESERVE: u64 = 54;
const VM_COMMIT: u64 = 55;
const VM_RELEASE: u64 = 57;
const PARK: u64 = 58;
const WAKE: u64 = 60;

fn call3(mut call: u64, first: u64, second: u64, mut third: u64) -> (i64, u64) {
    // Experimental native ABI zero preserves ordinary registers and FP state;
    // status and value are independent, even for high-bit successful values.
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inout("rax") call,
            inout("rdi") first => _,
            inout("rsi") second => _,
            inout("rdx") third,
            lateout("r8") _, lateout("r9") _, lateout("r10") _, lateout("r11") _,
        );
    }
    (call as i64, third)
}

pub(crate) fn park(word: &core::sync::atomic::AtomicU32) {
    match call3(PARK, word.as_ptr() as u64, 2, 0).0 {
        0 => {}
        // Exhausted wait credits do not invalidate the mutex. Yield and retry
        // its atomic predicate rather than turning contention into process OOM.
        -5 => {
            if call3(0, 0, 0, 0).0 != 0 {
                abort();
            }
        }
        _ => abort(),
    }
}

pub(crate) fn wake(word: &core::sync::atomic::AtomicU32) {
    if call3(WAKE, word.as_ptr() as u64, 1, 0).0 != 0 {
        abort();
    }
}

pub(crate) fn abort() -> ! {
    // An invariant failure cannot unwind through C or continue after losing a
    // backing owner. Kernel process exit performs the remaining VM retirement.
    unsafe {
        core::arch::asm!("int 0x80", "ud2", in("rax") 1u64, in("rdi") 127u64, options(noreturn));
    }
}

#[derive(Debug)]
pub(crate) struct PageAllocator;

// Talc's source requests byte-aligned backing in page-multiple blocks. Native
// reserve owns the virtual interval and commit supplies zeroed private RW/NX
// pages without calling any libc allocator or TLS-dependent function.
unsafe impl GlobalAlloc for PageAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let Some(bytes) = layout
            .size()
            .checked_add(PAGE_BYTES - 1)
            .map(|bytes| bytes & !(PAGE_BYTES - 1))
        else {
            return ptr::null_mut();
        };
        if bytes == 0 || bytes > isize::MAX as usize {
            return ptr::null_mut();
        }
        let (status, base) = call3(
            VM_RESERVE,
            bytes as u64,
            layout.align().max(PAGE_BYTES) as u64,
            0,
        );
        if status != 0 {
            return ptr::null_mut();
        }
        if call3(VM_COMMIT, base, bytes as u64, 0).0 != 0 {
            if call3(VM_RELEASE, base, 0, 0).0 != 0 {
                abort();
            }
            return ptr::null_mut();
        }
        base as *mut u8
    }

    unsafe fn dealloc(&self, data: *mut u8, _layout: Layout) {
        if call3(VM_RELEASE, data as u64, 0, 0).0 != 0 {
            abort();
        }
    }
}
