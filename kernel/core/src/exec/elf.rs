use crate::mm::frame::FRAME_SIZE;
use crate::mm::vm::Permissions;

const HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const MAX_PROGRAM_HEADERS: usize = 64;
const USER_END: u64 = 0x0000_8000_0000_0000;
const PT_GNU_STACK: u32 = 0x6474_e551;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadLimits {
    pub max_pages: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElfError {
    InvalidHeader,
    UnsupportedFormat,
    ProgramHeaders,
    UnsupportedSegment,
    InvalidSegment,
    InvalidAlignment,
    InvalidPermissions,
    InvalidAddress,
    OverlappingPages,
    SegmentCapacity,
    PageLimit,
    NoSegments,
    InvalidEntry,
}

/// A borrowed plan for a static ELF64 AMD64 executable. It does not allocate,
/// publish mappings, interpret section tables, or apply relocations.
#[derive(Debug)]
pub struct ElfImage<'file, const SEGMENTS: usize> {
    entry: u64,
    segments: [Option<LoadSegment<'file>>; SEGMENTS],
    segment_count: usize,
    page_count: usize,
}

impl<'file, const SEGMENTS: usize> ElfImage<'file, SEGMENTS> {
    pub fn parse(bytes: &'file [u8], limits: LoadLimits) -> Result<Self, ElfError> {
        let header = bytes.get(..HEADER_SIZE).ok_or(ElfError::InvalidHeader)?;
        if &header[..4] != b"\x7fELF" || header[6] != 1 || half(header, 52) != 64 {
            return Err(ElfError::InvalidHeader);
        }
        if header[4] != 2
            || header[5] != 1
            || header[7] != 0
            || header[8] != 0
            || half(header, 16) != 2
            || half(header, 18) != 62
            || word(header, 20) != 1
            || word(header, 48) != 0
        {
            return Err(ElfError::UnsupportedFormat);
        }
        let count = usize::from(half(header, 56));
        let offset = usize::try_from(double(header, 32)).map_err(|_| ElfError::ProgramHeaders)?;
        if count == 0
            || count > MAX_PROGRAM_HEADERS
            || offset < HEADER_SIZE
            || !offset.is_multiple_of(8)
            || usize::from(half(header, 54)) != PROGRAM_HEADER_SIZE
        {
            return Err(ElfError::ProgramHeaders);
        }
        let size = count
            .checked_mul(PROGRAM_HEADER_SIZE)
            .ok_or(ElfError::ProgramHeaders)?;
        let end = offset.checked_add(size).ok_or(ElfError::ProgramHeaders)?;
        let headers = bytes.get(offset..end).ok_or(ElfError::ProgramHeaders)?;
        let mut image = Self {
            entry: double(header, 24),
            segments: [None; SEGMENTS],
            segment_count: 0,
            page_count: 0,
        };
        for header in headers.as_chunks::<PROGRAM_HEADER_SIZE>().0 {
            match word(header, 0) {
                0 => {}
                1 => {
                    if let Some(segment) = LoadSegment::parse(header, bytes)? {
                        image.add_segment(segment, limits)?;
                    }
                }
                4 | 6 => {
                    // Notes and the PHDR description do not add mappings.
                    file_range(bytes, double(header, 8), double(header, 32))?;
                }
                PT_GNU_STACK => {
                    if word(header, 4) & !6 != 0 {
                        return Err(ElfError::InvalidPermissions);
                    }
                }
                // Dynamic linking, TLS templates, and other extensions need
                // explicit loader support before their headers can be accepted.
                _ => return Err(ElfError::UnsupportedSegment),
            }
        }
        if image.segment_count == 0 {
            return Err(ElfError::NoSegments);
        }
        if !image.segments().any(|segment| {
            segment.permissions.executable()
                && image.entry >= segment.address
                && image.entry - segment.address < segment.contents.len() as u64
        }) {
            return Err(ElfError::InvalidEntry);
        }
        Ok(image)
    }

    pub const fn entry(&self) -> u64 {
        self.entry
    }

    pub const fn page_count(&self) -> usize {
        self.page_count
    }

    pub fn segments(&self) -> impl ExactSizeIterator<Item = &LoadSegment<'file>> {
        self.segments[..self.segment_count]
            .iter()
            .map(|segment| segment.as_ref().expect("ELF segment plan invariant"))
    }

    pub fn pages(&self) -> impl Iterator<Item = LoadPage<'file>> {
        self.segments().flat_map(|segment| {
            let segment = *segment;
            (segment.page_start..segment.page_end)
                .step_by(FRAME_SIZE as usize)
                .map(move |address| segment.page(address))
        })
    }

    fn add_segment(
        &mut self,
        segment: LoadSegment<'file>,
        limits: LoadLimits,
    ) -> Result<(), ElfError> {
        if self.segment_count == SEGMENTS {
            return Err(ElfError::SegmentCapacity);
        }
        if self
            .segments()
            .any(|other| segment.page_start < other.page_end && other.page_start < segment.page_end)
        {
            return Err(ElfError::OverlappingPages);
        }
        let pages = usize::try_from((segment.page_end - segment.page_start) / FRAME_SIZE)
            .map_err(|_| ElfError::PageLimit)?;
        let page_count = self
            .page_count
            .checked_add(pages)
            .filter(|count| *count <= limits.max_pages)
            .ok_or(ElfError::PageLimit)?;
        self.segments[self.segment_count] = Some(segment);
        self.segment_count += 1;
        self.page_count = page_count;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LoadSegment<'file> {
    address: u64,
    memory_len: u64,
    contents: &'file [u8],
    permissions: Permissions,
    page_start: u64,
    page_end: u64,
}

impl<'file> LoadSegment<'file> {
    pub const fn address(&self) -> u64 {
        self.address
    }

    pub const fn memory_len(&self) -> u64 {
        self.memory_len
    }

    pub const fn permissions(&self) -> Permissions {
        self.permissions
    }

    pub const fn page_start(&self) -> u64 {
        self.page_start
    }

    pub const fn page_end(&self) -> u64 {
        self.page_end
    }

    fn parse(header: &[u8], file: &'file [u8]) -> Result<Option<Self>, ElfError> {
        let flags = word(header, 4);
        if flags & !7 != 0 || flags & 4 == 0 {
            return Err(ElfError::InvalidPermissions);
        }
        let permissions =
            Permissions::new(flags & 2 != 0, flags & 1 != 0).ok_or(ElfError::InvalidPermissions)?;
        let offset = double(header, 8);
        let address = double(header, 16);
        let file_len = double(header, 32);
        let memory_len = double(header, 40);
        let alignment = double(header, 48);
        if file_len > memory_len {
            return Err(ElfError::InvalidSegment);
        }
        let contents = file_range(file, offset, file_len)?;
        if address % FRAME_SIZE != offset % FRAME_SIZE
            || (alignment > 1
                && (!alignment.is_power_of_two() || address % alignment != offset % alignment))
        {
            return Err(ElfError::InvalidAlignment);
        }
        let end = address
            .checked_add(memory_len)
            .ok_or(ElfError::InvalidAddress)?;
        if !(FRAME_SIZE..USER_END).contains(&address) || end > USER_END {
            return Err(ElfError::InvalidAddress);
        }
        if memory_len == 0 {
            return Ok(None);
        }
        let page_start = address & !(FRAME_SIZE - 1);
        let page_end = end
            .checked_add(FRAME_SIZE - 1)
            .ok_or(ElfError::InvalidAddress)?
            & !(FRAME_SIZE - 1);
        Ok(Some(Self {
            address,
            memory_len,
            contents,
            permissions,
            page_start,
            page_end,
        }))
    }

    fn page(self, address: u64) -> LoadPage<'file> {
        let start = address.max(self.address);
        let end = (address + FRAME_SIZE).min(self.address + self.contents.len() as u64);
        let (contents, offset) = if start < end {
            let source = (start - self.address) as usize;
            let length = (end - start) as usize;
            (
                &self.contents[source..source + length],
                (start - address) as usize,
            )
        } else {
            (&[][..], 0)
        };
        LoadPage {
            address,
            permissions: self.permissions,
            contents,
            offset,
        }
    }
}

/// One complete page, including zero-filled padding and BSS. The loader must
/// reject reserved startup/stack ranges before publishing any planned page.
#[derive(Clone, Copy, Debug)]
pub struct LoadPage<'file> {
    address: u64,
    permissions: Permissions,
    contents: &'file [u8],
    offset: usize,
}

impl LoadPage<'_> {
    pub const fn address(&self) -> u64 {
        self.address
    }

    pub const fn permissions(&self) -> Permissions {
        self.permissions
    }

    pub fn initialize(&self, output: &mut [u8; FRAME_SIZE as usize]) {
        output.fill(0);
        output[self.offset..self.offset + self.contents.len()].copy_from_slice(self.contents);
    }
}

fn file_range(file: &[u8], offset: u64, length: u64) -> Result<&[u8], ElfError> {
    let end = offset.checked_add(length).ok_or(ElfError::InvalidSegment)?;
    let start = usize::try_from(offset).map_err(|_| ElfError::InvalidSegment)?;
    let end = usize::try_from(end).map_err(|_| ElfError::InvalidSegment)?;
    file.get(start..end).ok_or(ElfError::InvalidSegment)
}

// Callers pass a complete fixed-size ELF or program header and constant fields.
fn half(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn double(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
