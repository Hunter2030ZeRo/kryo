use crate::sync::atomic::{Atomic, Ordering};
use crate::sys::pal::native;
use crate::time::Duration;

pub type Futex = Atomic<Primitive>;
pub type Primitive = u32;
pub type SmallFutex = Atomic<SmallPrimitive>;
pub type SmallPrimitive = u32;

pub fn futex_wait(futex: &Futex, expected: Primitive, timeout: Option<Duration>) -> bool {
    if futex.load(Ordering::Relaxed) != expected {
        return true;
    }
    // As in the upstream backend, an overflowing deadline is an unbounded wait.
    let deadline = timeout.and_then(|duration| {
        u64::try_from(duration.as_nanos())
            .ok()
            .and_then(|nanos| native::monotonic_nanos().checked_add(nanos))
    });
    unsafe {
        native::__cryolite_rust_wait(
            futex.as_ptr(),
            expected,
            deadline.unwrap_or(0),
            deadline.is_some().into(),
        ) != native::TIMED_OUT
    }
}

pub fn futex_wake(futex: &Futex) -> bool {
    unsafe { native::__cryolite_rust_wake(futex.as_ptr(), 1) != 0 }
}

pub fn futex_wake_all(futex: &Futex) {
    unsafe { native::__cryolite_rust_wake(futex.as_ptr(), u64::MAX) };
}
