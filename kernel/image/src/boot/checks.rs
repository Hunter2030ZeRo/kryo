use super::memory::BootFramePool;
use kryo_core::mm::frame::{FRAME_SIZE, FrameAllocError, OwnedFrame, PhysRange};
use kryo_hal::{BootPageSlot, DirectMap};

const SCRATCH_ADDRESS: u64 = 0xffff_a000_0000_0000;
const BRANCH_CYCLES: u64 = 8;
const MAPPINGS_PER_BRANCH: u64 = 16;
const WORDS_PER_FRAME: usize = FRAME_SIZE as usize / size_of::<u64>();

pub(super) fn run(pool: &BootFramePool, direct_map: DirectMap) {
    check_frames(pool);
    check_ram(pool, direct_map);
    check_paging(pool, direct_map);
}

pub(super) fn check_high_memory(
    pool: &BootFramePool,
    direct: DirectMap,
    ranges: [Option<PhysRange>; 2],
) {
    for (minimum, range) in [1 << 32, 1 << 37].into_iter().zip(ranges) {
        let Some(range) = range else { continue };
        let before = pool.stats();
        let frame = pool
            .allocate_in(range)
            .expect("Cannot allocate high RAM frame");
        let physical = frame.address();
        assert!(physical >= minimum);
        let tables = allocate_frames::<3>(pool).expect("Cannot allocate high probe tables");
        let addresses = tables.each_ref().map(|frame| frame.address());
        // Table and data owners pin all aliases through the complete scratch retirement.
        let mut slot = unsafe { BootPageSlot::new(direct, SCRATCH_ADDRESS, addresses) }
            .expect("Cannot attach high RAM scratch slot");
        unsafe {
            direct.zero_frame(physical).expect("Cannot zero high RAM");
            assert!(direct.frame_is_zero(physical).unwrap());
            slot.map(physical).expect("Cannot map high RAM");
            check_alias(&slot, direct, physical, minimum);
        }
        assert_eq!(slot.unmap().expect("Cannot unmap high RAM"), physical);
        slot.detach().expect("Cannot detach high RAM scratch slot");
        unsafe {
            direct
                .zero_frame(physical)
                .expect("Cannot clear high RAM pattern");
            pool.release(frame).expect("Cannot release high RAM frame");
            release_frames(pool, tables);
        }
        let reused = pool
            .allocate_in(range)
            .expect("Cannot reuse high RAM frame");
        assert_eq!(reused.address(), physical);
        // The previous scratch branch and translation were retired before reuse.
        unsafe {
            assert!(direct.frame_is_zero(reused.address()).unwrap());
            pool.release(reused)
                .expect("Cannot return reused high RAM frame");
        }
        assert_eq!(pool.stats(), before);
        let _ = kryo_hal::print(format_args!(
            "RAM: high frame OK minimum={:#x} physical={:#x}\n",
            minimum, physical,
        ));
    }
}

fn check_frames(pool: &BootFramePool) {
    let before = pool.stats();
    let frames = allocate_frames::<2>(pool).expect("Cannot allocate test frames");
    assert_ne!(frames[0].address(), frames[1].address());
    assert!(
        frames
            .iter()
            .all(|frame| frame.address().is_multiple_of(FRAME_SIZE))
    );
    assert_eq!(pool.stats().free, before.free - 2);
    // These frames were never accessed or exposed to another owner.
    unsafe { release_frames(pool, frames) };
    assert_eq!(pool.stats(), before);
    let _ = kryo_hal::print(format_args!(
        "FRAME: alloc/release OK total={} free={}\n",
        before.total, before.free,
    ));
}

fn check_ram(pool: &BootFramePool, direct_map: DirectMap) {
    let before = pool.stats();
    let [frame] = allocate_frames::<1>(pool).expect("Cannot allocate RAM test frame");
    let address = frame.address();
    let pointer = direct_map
        .frame_address(address)
        .expect("Invalid RAM address") as *mut u64;
    // The retained token owns this RAM; no other context or device can access it.
    unsafe {
        for index in 0..WORDS_PER_FRAME {
            pointer.add(index).write_volatile(u64::MAX);
        }
        direct_map
            .zero_frame(address)
            .expect("Cannot clear RAM frame");
        assert!(
            direct_map
                .frame_is_zero(address)
                .expect("Cannot inspect RAM frame")
        );
        pool.release(frame).expect("Cannot release RAM test frame");
    }
    assert_eq!(pool.stats(), before);
    kryo_hal::emergency_write_str("RAM: zero OK\n");
}

fn check_paging(pool: &BootFramePool, direct_map: DirectMap) {
    let before = pool.stats();
    for branch in 0..BRANCH_CYCLES {
        let frames = allocate_frames::<5>(pool).expect("Cannot allocate paging test frames");
        let tables = [
            frames[0].address(),
            frames[1].address(),
            frames[2].address(),
        ];
        // Tokens pin all table frames until detach completes; BSP has IRQs disabled.
        let mut slot = unsafe { BootPageSlot::new(direct_map, SCRATCH_ADDRESS, tables) }
            .expect("Cannot attach scratch page tables");
        assert!(slot.unmap().is_err());
        check_busy_branch(pool, direct_map);

        for iteration in 0..MAPPINGS_PER_BRANCH {
            let physical = frames[3 + iteration as usize % 2].address();
            let other = frames[3 + (iteration as usize + 1) % 2].address();
            // Alternating backing frames exposes stale translations rather than reusing one page.
            unsafe {
                direct_map
                    .zero_frame(physical)
                    .expect("Cannot clear mapped frame");
                slot.map(physical).expect("Cannot map scratch page");
                assert!(slot.map(other).is_err());
                check_alias(
                    &slot,
                    direct_map,
                    physical,
                    branch * MAPPINGS_PER_BRANCH + iteration,
                );
            }
            assert_eq!(slot.unmap().expect("Cannot unmap scratch page"), physical);
            assert!(slot.unmap().is_err());
        }
        slot.detach().expect("Cannot detach scratch page tables");
        // The branch is unlinked and local translation caches have been flushed.
        unsafe { release_frames(pool, frames) };
        assert_eq!(pool.stats(), before);
    }
    let _ = kryo_hal::print(format_args!(
        "PAGING: map/unmap OK cycles={} free={}\n",
        BRANCH_CYCLES * MAPPINGS_PER_BRANCH,
        pool.stats().free,
    ));
}

fn check_busy_branch(pool: &BootFramePool, direct_map: DirectMap) {
    let before = pool.stats();
    let tables = allocate_frames::<3>(pool).expect("Cannot allocate rejected branch frames");
    let addresses = [
        tables[0].address(),
        tables[1].address(),
        tables[2].address(),
    ];
    // An occupied slot must be rejected before the supplied, unused frames are touched.
    let rejected = unsafe { BootPageSlot::new(direct_map, SCRATCH_ADDRESS, addresses) };
    assert!(rejected.is_err());
    // A rejected constructor never publishes these frames.
    unsafe { release_frames(pool, tables) };
    assert_eq!(pool.stats(), before);
}

unsafe fn check_alias(slot: &BootPageSlot, direct_map: DirectMap, physical: u64, seed: u64) {
    let mapped = slot.virtual_address() as *mut u64;
    let direct = direct_map
        .frame_address(physical)
        .expect("Invalid direct alias") as *const u64;
    for index in 0..WORDS_PER_FRAME {
        let value = 0x4b52_594f_0000_0000 ^ (seed << 16) ^ index as u64;
        // Both aliases name the owned frame and no Rust references into it are retained.
        unsafe {
            mapped.add(index).write_volatile(value);
            assert_eq!(direct.add(index).read_volatile(), value);
        }
    }
}

fn allocate_frames<const COUNT: usize>(
    pool: &BootFramePool,
) -> Result<[OwnedFrame<'_>; COUNT], FrameAllocError> {
    let mut frames = [const { None }; COUNT];
    for index in 0..COUNT {
        match pool.allocate() {
            Ok(frame) => frames[index] = Some(frame),
            Err(error) => {
                for frame in frames.into_iter().flatten() {
                    // Preparation failed before any frame was accessed or published.
                    unsafe { pool.release(frame) }.expect("Cannot roll back frame allocation");
                }
                return Err(error);
            }
        }
    }
    Ok(frames.map(|frame| frame.expect("Missing prepared frame")))
}

unsafe fn release_frames<const COUNT: usize>(
    pool: &BootFramePool,
    frames: [OwnedFrame<'_>; COUNT],
) {
    for frame in frames {
        unsafe { pool.release(frame) }.expect("Cannot release test frame");
    }
}
