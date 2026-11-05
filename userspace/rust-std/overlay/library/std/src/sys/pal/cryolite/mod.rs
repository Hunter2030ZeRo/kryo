#![deny(unsafe_op_in_unsafe_fn)]

pub(crate) mod native;

use crate::io;

pub unsafe fn init(argc: isize, argv: *const *const u8, _sigpipe: u8) {
    unsafe { crate::sys::args::init(argc, argv) };
}

pub unsafe fn cleanup() {
    unsafe { crate::sys::thread_local::destructors::run() };
    crate::rt::thread_cleanup();
}

pub fn unsupported<T>() -> io::Result<T> {
    Err(unsupported_err())
}

pub fn unsupported_err() -> io::Error {
    io::Error::UNSUPPORTED_PLATFORM
}

pub fn abort_internal() -> ! {
    unsafe { native::__cryolite_rust_abort() }
}
