pub fn fill_bytes(bytes: &mut [u8]) {
    for chunk in bytes.chunks_mut(256) {
        let error = unsafe {
            crate::sys::pal::native::__cryolite_rust_random(chunk.as_mut_ptr(), chunk.len())
        };
        if error != 0 {
            panic!(
                "Cryolite CPU random source failed: {}",
                crate::io::Error::from_raw_os_error(error)
            );
        }
    }
}
