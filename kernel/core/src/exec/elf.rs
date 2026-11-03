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
    InvalidTls,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgramHeaders {
    pub address: u64,
    pub file_offset: u64,
    pub entry_size: u64,
    pub count: u64,
}

/// A borrowed plan for a static ELF64 AMD64 executable. It does not allocate,
/// publish mappings, interpret section tables, or apply relocations.
#[derive(Debug)]
pub struct ElfImage<'file, const SEGMENTS: usize> {
    entry: u64,
    segments: [Option<LoadSegment<'file>>; SEGMENTS],
    segment_count: usize,
    page_count: usize,
    elf_header_address: Option<u64>,
    program_headers: Option<ProgramHeaders>,
    tls: Option<TlsSegment<'file>>,
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
            elf_header_address: None,
            program_headers: None,
            tls: None,
        };
        let mut phdr_address = None;
        let mut seen_load = false;
        for header in headers.as_chunks::<PROGRAM_HEADER_SIZE>().0 {
            match word(header, 0) {
                0 => {}
                1 => {
                    seen_load = true;
                    if let Some(segment) = LoadSegment::parse(header, bytes)? {
                        image.add_segment(segment, limits)?;
                    }
                }
                4 => {
                    file_range(bytes, double(header, 8), double(header, 32))?;
                }
                6 => {
                    if phdr_address.is_some()
                        || seen_load
                        || word(header, 4) != 4
                        || double(header, 8) != offset as u64
                        || double(header, 32) != size as u64
                        || double(header, 40) != size as u64
                    {
                        return Err(ElfError::ProgramHeaders);
                    }
                    let address = double(header, 16);
                    check_alignment(address, offset as u64, double(header, 48))
                        .map_err(|_| ElfError::ProgramHeaders)?;
                    phdr_address = Some(address);
                }
                7 => {
                    if image.tls.is_some() {
                        return Err(ElfError::InvalidTls);
                    }
                    image.tls = Some(TlsSegment::parse(header, bytes)?);
                }
                PT_GNU_STACK => {
                    if word(header, 4) & !6 != 0 {
                        return Err(ElfError::InvalidPermissions);
                    }
                }
                // Dynamic linking and other extensions need
                // explicit loader support before their headers can be accepted.
                _ => return Err(ElfError::UnsupportedSegment),
            }
        }
        if image.segment_count == 0 {
            return Err(ElfError::NoSegments);
        }
        image.elf_header_address = image.read_only_file_address(0, HEADER_SIZE as u64);
        image.program_headers = image
            .read_only_file_address(offset as u64, size as u64)
            .map(|address| ProgramHeaders {
                address,
                file_offset: offset as u64,
                entry_size: PROGRAM_HEADER_SIZE as u64,
                count: count as u64,
            });
        if let Some(address) = phdr_address
            && image.program_headers.map(|headers| headers.address) != Some(address)
        {
            return Err(ElfError::ProgramHeaders);
        }
        if let Some(tls) = image.tls {
            image.validate_tls(tls)?;
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

    pub const fn elf_header_address(&self) -> Option<u64> {
        self.elf_header_address
    }

    pub const fn program_headers(&self) -> Option<ProgramHeaders> {
        self.program_headers
    }

    pub const fn tls(&self) -> Option<TlsSegment<'file>> {
        self.tls
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

    fn read_only_file_address(&self, offset: u64, length: u64) -> Option<u64> {
        let mut address = None;
        for segment in self.segments() {
            if !segment.permissions.writable()
                && let Some(mapped) = segment.file_address(offset, length)
            {
                if address.is_some() {
                    return None;
                }
                address = Some(mapped);
            }
        }
        address
    }

    fn validate_tls(&self, tls: TlsSegment<'file>) -> Result<(), ElfError> {
        if tls.contents.is_empty() {
            return Ok(());
        }
        let mut backing = None;
        for segment in self.segments() {
            if let Some(address) = segment.file_address(tls.file_offset, tls.contents.len() as u64)
            {
                if backing.is_some() {
                    return Err(ElfError::InvalidTls);
                }
                backing = Some(address);
            }
        }
        if backing != Some(tls.address) {
            return Err(ElfError::InvalidTls);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LoadSegment<'file> {
    file_offset: u64,
    address: u64,
    memory_len: u64,
    contents: &'file [u8],
    permissions: Permissions,
    page_start: u64,
    page_end: u64,
}

impl<'file> LoadSegment<'file> {
    pub const fn file_offset(&self) -> u64 {
        self.file_offset
    }

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
        if address % FRAME_SIZE != offset % FRAME_SIZE {
            return Err(ElfError::InvalidAlignment);
        }
        check_alignment(address, offset, alignment)?;
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
            file_offset: offset,
            address,
            memory_len,
            contents,
            permissions,
            page_start,
            page_end,
        }))
    }

    fn file_address(&self, offset: u64, length: u64) -> Option<u64> {
        let relative = offset.checked_sub(self.file_offset)?;
        let end = relative.checked_add(length)?;
        if end > self.contents.len() as u64 {
            return None;
        }
        self.address.checked_add(relative)
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

/// Initialization bytes for one static TLS module. The zero-filled tail is
/// per-thread storage, not an additional mapping at the template address.
#[derive(Clone, Copy, Debug)]
pub struct TlsSegment<'file> {
    file_offset: u64,
    address: u64,
    memory_len: u64,
    alignment: u64,
    contents: &'file [u8],
}

impl<'file> TlsSegment<'file> {
    pub const fn address(&self) -> u64 {
        self.address
    }

    pub const fn memory_len(&self) -> u64 {
        self.memory_len
    }

    pub const fn alignment(&self) -> u64 {
        self.alignment
    }

    pub const fn contents(&self) -> &'file [u8] {
        self.contents
    }

    fn parse(header: &[u8], file: &'file [u8]) -> Result<Self, ElfError> {
        if word(header, 4) != 4 {
            return Err(ElfError::InvalidPermissions);
        }
        let file_offset = double(header, 8);
        let address = double(header, 16);
        let file_len = double(header, 32);
        let memory_len = double(header, 40);
        if file_len > memory_len {
            return Err(ElfError::InvalidTls);
        }
        let alignment = double(header, 48);
        check_alignment(address, file_offset, alignment)?;
        let end = address
            .checked_add(memory_len)
            .ok_or(ElfError::InvalidAddress)?;
        if address >= USER_END || end > USER_END || (memory_len != 0 && address < FRAME_SIZE) {
            return Err(ElfError::InvalidAddress);
        }
        Ok(Self {
            file_offset,
            address,
            memory_len,
            alignment: alignment.max(1),
            contents: file_range(file, file_offset, file_len)?,
        })
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

fn check_alignment(address: u64, offset: u64, alignment: u64) -> Result<(), ElfError> {
    if alignment > 1 && (!alignment.is_power_of_two() || address % alignment != offset % alignment)
    {
        return Err(ElfError::InvalidAlignment);
    }
    Ok(())
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
