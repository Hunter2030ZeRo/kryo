use crate::sys::pal::native;

pub type Key = usize;

pub fn create(destructor: Option<unsafe extern "C" fn(*mut u8)>) -> Key {
    let mut key = 0;
    if unsafe { native::__cryolite_rust_key_create(&mut key, destructor) } != 0 {
        rtabort!("out of TLS keys");
    }
    key
}

pub unsafe fn set(key: Key, value: *mut u8) {
    if unsafe { native::__cryolite_rust_key_set(key, value) } != 0 {
        rtabort!("TLS value registration failed");
    }
}

#[cfg(any(not(target_thread_local), test))]
pub unsafe fn get(key: Key) -> *mut u8 {
    unsafe { native::__cryolite_rust_key_get(key) }
}

pub unsafe fn destroy(key: Key) {
    unsafe { native::__cryolite_rust_key_delete(key) };
}
