use super::{BootFramePool, DirectMap, KernelMemory, KernelRegion, RefCell};
use crate::runtime::launch;
use crate::runtime::stacks::{KernelStack, StackError};
use kryo_hal::PagingError;
use kryo_interfaces::syscall::Error;

pub(super) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let valid = executable(0x40_0000);
    // HHDM split-table ownership is permanent and separate from a prepared
    // process. Warm the same allocation range before checking rollback counts.
    let mut prepared = match launch::prepare(pool, direct, memory, region, 2, &valid) {
        Ok(prepared) => prepared,
        Err(mut failure) => {
            assert!(failure.pending.close(pool, region));
            panic!("Valid launch fixture was rejected");
        }
    };
    prepared
        .close(pool, region)
        .expect("Cannot retire prepared launch fixture");
    drop(prepared);

    reject(
        pool,
        direct,
        memory,
        region,
        &[0; 64],
        Error::InvalidArgument,
        false,
    );
    reject(
        pool,
        direct,
        memory,
        region,
        &valid[..63],
        Error::InvalidArgument,
        false,
    );
    reject(
        pool,
        direct,
        memory,
        region,
        &valid[..4096],
        Error::InvalidArgument,
        false,
    );
    for address in [0x80_0000, 0x7f_f000, 0x90_0000] {
        reject(
            pool,
            direct,
            memory,
            region,
            &executable(address),
            Error::InvalidArgument,
            false,
        );
    }

    let occupied = pool.allocate().expect("No launch collision frame");
    unsafe {
        direct.zero_frame(occupied.address()).unwrap();
        region.map_ram(4, occupied.address()).unwrap();
    }
    reject(
        pool,
        direct,
        memory,
        region,
        &valid,
        Error::OutOfResources,
        true,
    );
    let before = pool.stats();
    let failure = match KernelStack::try_new(pool, direct, region, 2) {
        Ok(mut stack) => {
            unsafe { stack.close(pool, region) }.unwrap();
            panic!("Kernel stack collision was accepted");
        }
        Err(failure) => failure,
    };
    assert_eq!(
        failure.error,
        StackError::Paging(PagingError::AlreadyMapped)
    );
    assert!(failure.cleanup_error.is_none() && failure.retained.is_none());
    assert_eq!(pool.stats(), before);
    for slot in [2, 3, 5] {
        assert_eq!(region.permissions(slot), Err(PagingError::NotMapped));
    }
    assert!(region.permissions(4).unwrap().writable);
    unsafe {
        assert_eq!(region.unmap(4).unwrap(), occupied.address());
        pool.release(occupied).unwrap();
    }

    let before = pool.stats();
    let failure = match KernelStack::try_new(pool, direct, region, 0) {
        Ok(mut stack) => {
            unsafe { stack.close(pool, region) }.unwrap();
            panic!("Kernel stack without a guard was accepted");
        }
        Err(failure) => failure,
    };
    assert_eq!(
        failure.error,
        StackError::Paging(PagingError::InvalidAddress)
    );
    assert!(failure.cleanup_error.is_none() && failure.retained.is_none());
    assert_eq!(pool.stats(), before);
    kryo_hal::emergency_write_str("LAUNCH: rejection and rollback OK cases=9\n");
}

fn reject(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
    bytes: &[u8],
    expected: Error,
    retained_vm: bool,
) {
    let before = pool.stats();
    let mut failure = match launch::prepare(pool, direct, memory, region, 2, bytes) {
        Ok(mut prepared) => {
            prepared.close(pool, region).unwrap();
            panic!("Rejected launch fixture was accepted");
        }
        Err(failure) => failure,
    };
    assert_eq!(failure.error, expected);
    if retained_vm {
        assert!(pool.stats().free < before.free);
    } else {
        assert_eq!(pool.stats(), before);
    }
    assert!(failure.pending.close(pool, region));
    assert!(failure.pending.close(pool, region));
    assert_eq!(pool.stats(), before);
}

fn executable(address: u64) -> [u8; 4097] {
    let mut bytes = [0; 4097];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes[16..18].copy_from_slice(&2u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&62u16.to_le_bytes());
    bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
    bytes[24..32].copy_from_slice(&address.to_le_bytes());
    bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
    bytes[52..54].copy_from_slice(&64u16.to_le_bytes());
    bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
    bytes[56..58].copy_from_slice(&1u16.to_le_bytes());
    bytes[64..68].copy_from_slice(&1u32.to_le_bytes());
    bytes[68..72].copy_from_slice(&5u32.to_le_bytes());
    bytes[72..80].copy_from_slice(&4096u64.to_le_bytes());
    bytes[80..88].copy_from_slice(&address.to_le_bytes());
    bytes[96..104].copy_from_slice(&1u64.to_le_bytes());
    bytes[104..112].copy_from_slice(&1u64.to_le_bytes());
    bytes[112..120].copy_from_slice(&4096u64.to_le_bytes());
    bytes[4096] = 0xcc;
    bytes
}
