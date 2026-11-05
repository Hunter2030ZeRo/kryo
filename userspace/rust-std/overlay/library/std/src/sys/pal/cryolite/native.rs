use crate::ffi::{c_int, c_void};

pub(crate) const TIMED_OUT: i64 = -10;

unsafe extern "C" {
    pub(crate) fn __cryolite_rust_abort() -> !;
    pub(crate) fn __cryolite_rust_clock() -> u64;
    pub(crate) fn __cryolite_rust_sleep_until(deadline: u64);
    pub(crate) fn __cryolite_rust_yield();
    pub(crate) fn __cryolite_rust_wait(
        word: *const u32,
        expected: u32,
        deadline: u64,
        timed: c_int,
    ) -> i64;
    pub(crate) fn __cryolite_rust_wake(word: *const u32, maximum: u64) -> u64;

    pub(crate) fn __cryolite_rust_thread_create(
        thread: *mut usize,
        stack_size: usize,
        entry: unsafe extern "C" fn(*mut c_void) -> c_int,
        argument: *mut c_void,
    ) -> c_int;
    pub(crate) fn __cryolite_rust_thread_join(thread: usize) -> c_int;
    pub(crate) fn __cryolite_rust_thread_detach(thread: usize) -> c_int;

    pub(crate) fn __cryolite_rust_key_create(
        key: *mut usize,
        destructor: Option<unsafe extern "C" fn(*mut u8)>,
    ) -> c_int;
    pub(crate) fn __cryolite_rust_key_delete(key: usize);
    pub(crate) fn __cryolite_rust_key_get(key: usize) -> *mut u8;
    pub(crate) fn __cryolite_rust_key_set(key: usize, value: *mut u8) -> c_int;
}

#[inline]
pub(crate) fn monotonic_nanos() -> u64 {
    unsafe { __cryolite_rust_clock() }
}
