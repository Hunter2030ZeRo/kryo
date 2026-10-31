use super::{FRAME_SIZE, MemoryRegion, PhysRange, RegionKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalizeError {
    EmptyRegion,
    OverlappingRegions,
    InvalidRange,
    AddressOverflow,
    OutputTooSmall,
}

#[derive(Debug)]
pub struct MemoryPlan<'a> {
    available: &'a [PhysRange],
    frame_count: u64,
}

impl MemoryPlan<'_> {
    pub fn available(&self) -> &[PhysRange] {
        self.available
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }
}

pub fn normalize_memory<'a>(
    regions: &mut [MemoryRegion],
    reservations: &mut [PhysRange],
    output: &'a mut [PhysRange],
) -> Result<MemoryPlan<'a>, NormalizeError> {
    regions.sort_unstable_by_key(|region| region.range.start());

    if regions.iter().any(|region| region.range.is_empty()) {
        return Err(NormalizeError::EmptyRegion);
    }
    if regions
        .windows(2)
        .any(|pair| pair[0].range.end() > pair[1].range.start())
    {
        return Err(NormalizeError::OverlappingRegions);
    }

    let reserved_count = merge_reservations(reservations)?;
    let reservations = &reservations[..reserved_count];
    let mut reserved_index = 0;
    let mut written = 0;

    for region in regions.iter() {
        if region.kind != RegionKind::Usable {
            continue;
        }
        let Some(area) = region.range.page_interior() else {
            continue;
        };
        let mut cursor = area.start();

        while reserved_index < reservations.len() && reservations[reserved_index].end() <= cursor {
            reserved_index += 1;
        }

        while reserved_index < reservations.len()
            && reservations[reserved_index].start() < area.end()
        {
            let reserved = reservations[reserved_index];

            if cursor < reserved.start() {
                append_available(output, &mut written, cursor, reserved.start())?;
            }

            cursor = cursor.max(reserved.end()).min(area.end());

            if reserved.end() <= area.end() {
                reserved_index += 1;
            }
            if cursor == area.end() {
                break;
            }
        }

        append_available(output, &mut written, cursor, area.end())?;
    }

    let available = &output[..written];
    let frame_count = available.iter().map(|range| range.len() / FRAME_SIZE).sum();
    Ok(MemoryPlan {
        available,
        frame_count,
    })
}

fn merge_reservations(reservations: &mut [PhysRange]) -> Result<usize, NormalizeError> {
    for range in reservations.iter_mut() {
        *range = range.page_cover().ok_or(NormalizeError::AddressOverflow)?;
    }

    reservations.sort_unstable_by_key(|range| range.start());
    let mut written = 0;

    for index in 0..reservations.len() {
        let current = reservations[index];
        if current.is_empty() {
            continue;
        }

        if written > 0 && current.start() <= reservations[written - 1].end() {
            let previous = reservations[written - 1];
            let end = previous.end().max(current.end());
            reservations[written - 1] =
                PhysRange::from_base_len(previous.start(), end - previous.start())
                    .ok_or(NormalizeError::AddressOverflow)?;
        } else {
            reservations[written] = current;
            written += 1;
        }
    }

    Ok(written)
}

fn append_available(
    output: &mut [PhysRange],
    written: &mut usize,
    start: u64,
    end: u64,
) -> Result<(), NormalizeError> {
    if start > end {
        return Err(NormalizeError::InvalidRange);
    }
    if start == end {
        return Ok(());
    }

    if *written > 0 && output[*written - 1].end() == start {
        let previous = output[*written - 1];
        output[*written - 1] = PhysRange::from_base_len(previous.start(), end - previous.start())
            .ok_or(NormalizeError::AddressOverflow)?;
        return Ok(());
    }

    let slot = output
        .get_mut(*written)
        .ok_or(NormalizeError::OutputTooSmall)?;
    *slot = PhysRange::from_base_len(start, end - start).ok_or(NormalizeError::AddressOverflow)?;
    *written += 1;
    Ok(())
}
