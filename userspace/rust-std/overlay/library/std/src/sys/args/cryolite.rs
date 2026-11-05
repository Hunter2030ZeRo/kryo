pub use super::common::Args;
use crate::ffi::{CStr, OsString};
use crate::os::unix::ffi::OsStringExt;
use crate::ptr;
use crate::sync::atomic::{AtomicIsize, AtomicPtr, Ordering};

static ARGC: AtomicIsize = AtomicIsize::new(0);
static ARGV: AtomicPtr<*const u8> = AtomicPtr::new(ptr::null_mut());

// The C startup owns this immutable argument storage for the process lifetime.
pub unsafe fn init(argc: isize, argv: *const *const u8) {
    ARGC.store(argc, Ordering::Relaxed);
    ARGV.store(argv.cast_mut(), Ordering::Release);
}

pub fn args() -> Args {
    let argv = ARGV.load(Ordering::Acquire);
    let argc = if argv.is_null() {
        0
    } else {
        ARGC.load(Ordering::Relaxed).max(0)
    };
    let mut values = Vec::with_capacity(argc as usize);
    for index in 0..argc {
        // SAFETY: init receives the process-lifetime, null-terminated SysV argv.
        let arg = unsafe { *argv.offset(index) };
        if arg.is_null() {
            break;
        }
        let bytes = unsafe { CStr::from_ptr(arg.cast()) }.to_bytes();
        values.push(OsString::from_vec(bytes.to_vec()));
    }
    Args::new(values)
}
