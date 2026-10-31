use kryo_core::mm::frame::{FRAME_SIZE, PhysRange};

fn range(start: u64, end: u64) -> PhysRange {
    PhysRange::from_base_len(start, end - start).unwrap()
}

#[test]
fn base_and_length_use_a_half_open_range() {
    let area = PhysRange::from_base_len(0x1234, 0x5000).unwrap();
    assert_eq!(area.start(), 0x1234);
    assert_eq!(area.end(), 0x6234);
    assert_eq!(area.len(), 0x5000);
    assert!(!area.is_empty());
    assert!(PhysRange::from_base_len(u64::MAX, 1).is_none());
}

#[test]
fn page_alignment_keeps_complete_frames_and_covers_partial_frames() {
    let area = range(0x1001, 0x4fff);
    assert_eq!(area.page_interior(), Some(range(0x2000, 0x4000)));
    assert_eq!(area.page_cover(), Some(range(0x1000, 0x5000)));

    let aligned = range(0x2000, 0x5000);
    assert_eq!(aligned.page_interior(), Some(aligned));
    assert_eq!(aligned.page_cover(), Some(aligned));
}

#[test]
fn short_and_empty_ranges_have_no_complete_frames() {
    assert!(range(0x1001, 0x2000).page_interior().is_none());
    assert!(range(0x1000, 0x1fff).page_interior().is_none());
    for address in [0, 0x1234, u64::MAX] {
        let empty = PhysRange::from_base_len(address, 0).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.page_interior(), None);
        assert_eq!(empty.page_cover(), Some(PhysRange::EMPTY));
    }
}

#[test]
fn top_of_address_space_rejects_unrepresentable_page_cover() {
    let last_aligned = u64::MAX & !(FRAME_SIZE - 1);
    let partial = range(last_aligned, u64::MAX);
    assert_eq!(partial.page_interior(), None);
    assert_eq!(partial.page_cover(), None);

    let complete = range(last_aligned - FRAME_SIZE, last_aligned);
    assert_eq!(complete.page_interior(), Some(complete));
    assert_eq!(complete.page_cover(), Some(complete));
}

#[test]
fn alignment_matches_integer_arithmetic_across_page_boundaries() {
    for start in 0..3 * FRAME_SIZE {
        for length in [0, 1, FRAME_SIZE - 1, FRAME_SIZE, FRAME_SIZE + 1] {
            let end = start + length;
            let area = range(start, end);
            let first_page = start.div_ceil(FRAME_SIZE) * FRAME_SIZE;
            let last_page = end / FRAME_SIZE * FRAME_SIZE;
            let expected = (first_page < last_page).then(|| range(first_page, last_page));
            assert_eq!(area.page_interior(), expected);

            if length > 0 {
                assert_eq!(
                    area.page_cover(),
                    Some(range(
                        start / FRAME_SIZE * FRAME_SIZE,
                        end.div_ceil(FRAME_SIZE) * FRAME_SIZE,
                    ))
                );
            }
        }
    }
}
