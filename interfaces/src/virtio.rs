//! Bounded native transport records. Offsets name a retained memory object,
//! never CPU pointers, physical frames or device addresses.

pub const MAX_BUFFERS: usize = 3;
pub const DEVICE_WRITE: u32 = 1;
pub const COMPLETE: u64 = 1 << 63;
pub const VERSION_1: u64 = 1 << 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Buffer {
    pub offset: u64,
    pub length: u32,
    pub flags: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_have_fixed_width_and_alignment() {
        assert_eq!(core::mem::size_of::<Buffer>(), 16);
        assert_eq!(core::mem::align_of::<Buffer>(), 8);
    }
}
