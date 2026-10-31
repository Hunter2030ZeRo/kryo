use std::collections::BTreeSet;

use kryo_core::mm::frame::{
    FRAME_SIZE, MemoryRegion, NormalizeError, PhysRange, RegionKind, normalize_memory,
};

fn range(start: u64, end: u64) -> PhysRange {
    PhysRange::from_base_len(start, end - start).unwrap()
}

fn region(start: u64, end: u64, kind: RegionKind) -> MemoryRegion {
    MemoryRegion {
        range: range(start, end),
        kind,
    }
}

fn usable(start: u64, end: u64) -> MemoryRegion {
    region(start, end, RegionKind::Usable)
}

#[test]
fn empty_input_and_fully_reserved_maps_need_no_output() {
    let plan = normalize_memory(&mut [], &mut [], &mut []).unwrap();
    assert!(plan.available().is_empty());
    assert_eq!(plan.frame_count(), 0);

    let mut regions = [usable(0x1000, 0x5000)];
    let mut reservations = [range(0, 0x6000)];
    let plan = normalize_memory(&mut regions, &mut reservations, &mut []).unwrap();
    assert!(plan.available().is_empty());
    assert_eq!(plan.frame_count(), 0);
}

#[test]
fn non_usable_kinds_never_become_allocation_candidates() {
    let excluded = [
        RegionKind::Reserved,
        RegionKind::AcpiReclaimable,
        RegionKind::AcpiNvs,
        RegionKind::BadMemory,
        RegionKind::BootloaderReclaimable,
        RegionKind::ExecutableAndModules,
        RegionKind::Framebuffer,
        RegionKind::MappedReserved,
        RegionKind::Unknown(0),
        RegionKind::Unknown(u64::MAX),
    ];
    let mut regions: Vec<_> = excluded
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            let start = index as u64 * FRAME_SIZE;
            region(start, start + FRAME_SIZE, kind)
        })
        .collect();
    regions.push(usable(0x10001, 0x15001));
    regions.reverse();
    let mut output = [PhysRange::EMPTY; 1];
    let plan = normalize_memory(&mut regions, &mut [], &mut output).unwrap();
    assert_eq!(plan.available(), &[range(0x11000, 0x15000)]);
    assert_eq!(plan.frame_count(), 4);
}

#[test]
fn reservations_round_outward_and_union_overlaps_and_adjacency() {
    let mut regions = [usable(0x1001, 0xcfff)];
    let mut reservations = [
        range(0x8001, 0x8fff),
        range(0x5001, 0x6001),
        range(0x6000, 0x8000),
        range(0x3001, 0x4000),
        PhysRange::EMPTY,
    ];
    let mut output = [PhysRange::EMPTY; 4];
    let plan = normalize_memory(&mut regions, &mut reservations, &mut output).unwrap();
    assert_eq!(
        plan.available(),
        &[
            range(0x2000, 0x3000),
            range(0x4000, 0x5000),
            range(0x9000, 0xc000),
        ]
    );
    assert_eq!(plan.frame_count(), 5);
}

#[test]
fn one_reservation_can_span_several_usable_regions() {
    let mut regions = [
        usable(0x1000, 0x4000),
        region(0x4000, 0x8000, RegionKind::Reserved),
        usable(0x8000, 0xa000),
        usable(0xc000, 0x10000),
    ];
    let mut reservations = [range(0x2001, 0xd001)];
    let mut output = [PhysRange::EMPTY; 2];
    let plan = normalize_memory(&mut regions, &mut reservations, &mut output).unwrap();
    assert_eq!(
        plan.available(),
        &[range(0x1000, 0x2000), range(0xe000, 0x10000)]
    );
    assert_eq!(plan.frame_count(), 3);
}

#[test]
fn adjacent_usable_regions_merge_even_when_output_is_full() {
    let mut regions = [usable(0x3000, 0x5000), usable(0x1000, 0x3000)];
    let mut output = [PhysRange::EMPTY; 1];
    let plan = normalize_memory(&mut regions, &mut [], &mut output).unwrap();
    assert_eq!(plan.available(), &[range(0x1000, 0x5000)]);
    assert_eq!(plan.frame_count(), 4);
}

#[test]
fn touching_reservations_outside_a_region_do_not_remove_its_pages() {
    let mut regions = [usable(0x4000, 0x8000)];
    let mut reservations = [range(0, 0x4000), range(0x8000, 0x10000)];
    let mut output = [PhysRange::EMPTY; 1];
    let plan = normalize_memory(&mut regions, &mut reservations, &mut output).unwrap();
    assert_eq!(plan.available(), &[range(0x4000, 0x8000)]);
    assert_eq!(plan.frame_count(), 4);
}

#[test]
fn regions_without_complete_pages_produce_no_candidates() {
    let mut regions = [usable(0x1001, 0x2000), usable(0x3000, 0x3fff)];
    let plan = normalize_memory(&mut regions, &mut [], &mut []).unwrap();
    assert!(plan.available().is_empty());
    assert_eq!(plan.frame_count(), 0);
}

#[test]
fn invalid_raw_maps_are_rejected_instead_of_partially_accepted() {
    let mut output = [PhysRange::EMPTY; 4];
    let mut empty = [usable(0x1000, 0x1000)];
    assert_eq!(
        normalize_memory(&mut empty, &mut [], &mut output).unwrap_err(),
        NormalizeError::EmptyRegion
    );

    for kind in [RegionKind::Usable, RegionKind::Reserved] {
        let mut overlap = [usable(0x1000, 0x4000), region(0x3000, 0x5000, kind)];
        assert_eq!(
            normalize_memory(&mut overlap, &mut [], &mut output).unwrap_err(),
            NormalizeError::OverlappingRegions
        );
    }
}

#[test]
fn reservation_cover_overflow_is_an_error_even_outside_usable_ram() {
    let mut regions = [usable(0x1000, 0x5000)];
    let mut reservations = [range(u64::MAX - 1, u64::MAX)];
    let mut output = [PhysRange::EMPTY; 1];
    assert_eq!(
        normalize_memory(&mut regions, &mut reservations, &mut output).unwrap_err(),
        NormalizeError::AddressOverflow
    );
}

#[test]
fn output_capacity_failure_cannot_be_mistaken_for_a_complete_plan() {
    let mut regions = [usable(0x1000, 0x8000)];
    let mut reservations = [range(0x4000, 0x5000)];
    let mut output = [PhysRange::EMPTY; 1];
    assert_eq!(
        normalize_memory(&mut regions, &mut reservations, &mut output).unwrap_err(),
        NormalizeError::OutputTooSmall
    );
}

fn next_random(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    *seed
}

fn expected_pages(regions: &[MemoryRegion], reservations: &[PhysRange]) -> BTreeSet<u64> {
    let end = regions
        .iter()
        .map(|region| region.range.end())
        .max()
        .unwrap_or(0);
    (0..end.div_ceil(FRAME_SIZE))
        .map(|index| index * FRAME_SIZE)
        .filter(|&page| {
            regions.iter().any(|region| {
                region.kind == RegionKind::Usable
                    && region.range.start() <= page
                    && page + FRAME_SIZE <= region.range.end()
            }) && !reservations.iter().any(|reserved| {
                !reserved.is_empty()
                    && page < reserved.end()
                    && reserved.start() < page + FRAME_SIZE
            })
        })
        .collect()
}

#[test]
fn generated_maps_match_an_independent_page_oracle() {
    let mut seed = 0xcafef00du64;
    for case in 0..2_000 {
        let mut start = 0;
        let count = next_random(&mut seed) % 12;
        let mut regions = Vec::new();
        for _ in 0..count {
            start += next_random(&mut seed) % (2 * FRAME_SIZE);
            let end = start + 1 + next_random(&mut seed) % (8 * FRAME_SIZE);
            let kind = match next_random(&mut seed) % 5 {
                0..=2 => RegionKind::Usable,
                3 => RegionKind::BootloaderReclaimable,
                _ => RegionKind::Unknown(42),
            };
            regions.push(region(start, end, kind));
            start = end;
        }
        let mut reservations = Vec::new();
        for _ in 0..next_random(&mut seed) % 12 {
            let base = next_random(&mut seed) % (start + FRAME_SIZE);
            let length = next_random(&mut seed) % (12 * FRAME_SIZE);
            reservations.push(range(base, base + length));
        }
        let expected = expected_pages(&regions, &reservations);
        regions.reverse();
        reservations.reverse();
        let mut output = [PhysRange::EMPTY; 32];
        let plan = normalize_memory(&mut regions, &mut reservations, &mut output).unwrap();
        let actual: BTreeSet<_> = plan
            .available()
            .iter()
            .flat_map(|range| (range.start()..range.end()).step_by(FRAME_SIZE as usize))
            .collect();

        assert_eq!(actual, expected, "generated map {case}");
        assert_eq!(plan.frame_count(), expected.len() as u64);
        assert!(plan.available().iter().all(|range| {
            !range.is_empty() && range.start() % FRAME_SIZE == 0 && range.end() % FRAME_SIZE == 0
        }));
        assert!(
            plan.available()
                .windows(2)
                .all(|pair| pair[0].end() < pair[1].start())
        );
    }
}
