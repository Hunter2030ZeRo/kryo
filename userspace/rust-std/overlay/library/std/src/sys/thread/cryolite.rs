use crate::ffi::{CStr, c_int, c_void};
use crate::io;
use crate::mem::ManuallyDrop;
use crate::num::NonZero;
use crate::sys::pal::native;
use crate::thread::ThreadInit;
use crate::time::Duration;

pub struct Thread {
    id: usize,
}

pub const DEFAULT_MIN_STACK_SIZE: usize = 2 * 1024 * 1024;

impl Thread {
    pub unsafe fn new(stack: usize, init: Box<ThreadInit>) -> io::Result<Thread> {
        let argument = Box::into_raw(init);
        let mut id = 0;
        let error = unsafe {
            native::__cryolite_rust_thread_create(&mut id, stack, start, argument.cast())
        };
        if error != 0 {
            unsafe { drop(Box::from_raw(argument)) };
            return Err(io::Error::from_raw_os_error(error));
        }
        Ok(Thread { id })
    }

    pub fn join(self) {
        let thread = ManuallyDrop::new(self);
        let error = unsafe { native::__cryolite_rust_thread_join(thread.id) };
        assert_eq!(error, 0, "native thread join failed");
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn into_id(self) -> usize {
        ManuallyDrop::new(self).id
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        if unsafe { native::__cryolite_rust_thread_detach(self.id) } != 0 {
            rtabort!("native thread detach failed");
        }
    }
}

unsafe extern "C" fn start(argument: *mut c_void) -> c_int {
    let init = unsafe { Box::from_raw(argument.cast::<ThreadInit>()) };
    init.init()();
    // The provider runs the registered TLS guard before retiring its TCB.
    0
}

pub fn available_parallelism() -> io::Result<NonZero<usize>> {
    Err(io::Error::UNKNOWN_THREAD_COUNT)
}

pub fn current_os_id() -> Option<u64> {
    None
}

pub fn yield_now() {
    unsafe { native::__cryolite_rust_yield() };
}

pub fn set_name(_name: &CStr) {}

pub fn sleep(duration: Duration) {
    if duration.is_zero() {
        return;
    }
    let nanos = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
    let deadline = native::monotonic_nanos().saturating_add(nanos);
    unsafe { native::__cryolite_rust_sleep_until(deadline) };
}
