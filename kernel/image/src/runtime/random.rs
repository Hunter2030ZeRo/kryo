use kryo_interfaces::syscall::{Error, RANDOM_WORDS};

const _: () = assert!(RANDOM_WORDS * 8 == kryo_hal::CPU_RANDOM_MAX);

pub(super) fn words(first: u64, second: u64) -> Result<[u64; RANDOM_WORDS], Error> {
    if first != 0 || second != 0 {
        return Err(Error::InvalidArgument);
    }
    let mut bytes = [0; kryo_hal::CPU_RANDOM_MAX];
    // Native dispatch retains the BSP after all user CPU leases are quiescent.
    let result = unsafe { kryo_hal::fill_cpu_random(&mut bytes) };
    let words = result.map(|()| {
        [
            u64::from_ne_bytes(bytes[..8].try_into().unwrap()),
            u64::from_ne_bytes(bytes[8..].try_into().unwrap()),
        ]
    });
    for byte in &mut bytes {
        // No abandoned prefix is exposed after source failure.
        unsafe { core::ptr::write_volatile(byte, 0) };
    }
    words.map_err(|error| match error {
        kryo_hal::RandomError::Unsupported => Error::Unsupported,
        kryo_hal::RandomError::Busy => Error::Busy,
        kryo_hal::RandomError::InvalidLength => Error::InvalidArgument,
    })
}
