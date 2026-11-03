// Compile-only candidate checks; no public ABI type is installed here.
#[allow(dead_code)]
#[path = "../../../interfaces/src/virtio.rs"]
mod transport;

#[repr(C)]
struct DraftTimespec {
    tv_sec: i64,
    tv_nsec: i64,
}

const _: () = {
    assert!(core::mem::size_of::<usize>() == 8);
    assert!(core::mem::size_of::<isize>() == 8);
    assert!(core::mem::size_of::<i32>() == 4);
    assert!(core::mem::size_of::<DraftTimespec>() == 16);
    assert!(core::mem::align_of::<DraftTimespec>() == 8);
    assert!(core::mem::offset_of!(DraftTimespec, tv_sec) == 0);
    assert!(core::mem::offset_of!(DraftTimespec, tv_nsec) == 8);
    assert!(core::mem::size_of::<transport::Buffer>() == 16);
    assert!(core::mem::align_of::<transport::Buffer>() == 8);
    assert!(core::mem::offset_of!(transport::Buffer, offset) == 0);
    assert!(core::mem::offset_of!(transport::Buffer, length) == 8);
    assert!(core::mem::offset_of!(transport::Buffer, flags) == 12);
};
