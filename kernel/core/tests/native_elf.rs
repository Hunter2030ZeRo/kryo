use kryo_core::exec::{ElfError, ElfImage, LoadLimits};
use kryo_core::mm::vm::Permissions;

const TEXT: u64 = 0x40_0000;
const DATA: u64 = 0x60_0000;
const USER_END: u64 = 0x8000_0000_0000;
const PH: usize = 64;
const PH_SIZE: usize = 56;
const LIMITS: LoadLimits = LoadLimits { max_pages: 8 };

#[derive(Clone, Copy)]
struct Segment<'bytes> {
    kind: u32,
    flags: u32,
    offset: u64,
    address: u64,
    contents: &'bytes [u8],
    memory_len: u64,
    alignment: u64,
}

fn text() -> Segment<'static> {
    Segment {
        kind: 1,
        flags: 5,
        offset: 0x1000,
        address: TEXT,
        contents: &[0x90, 0x90, 0x0f, 0x0b],
        memory_len: 4,
        alignment: 4096,
    }
}

fn elf(segments: &[Segment<'_>], entry: u64) -> Vec<u8> {
    let length = segments
        .iter()
        .fold(PH + segments.len() * PH_SIZE, |end, segment| {
            end.max(segment.offset as usize + segment.contents.len())
        });
    let mut file = vec![0; length];
    file[..9].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0]);
    put16(&mut file, 16, 2);
    put16(&mut file, 18, 62);
    put32(&mut file, 20, 1);
    put64(&mut file, 24, entry);
    put64(&mut file, 32, PH as u64);
    put16(&mut file, 52, 64);
    put16(&mut file, 54, PH_SIZE as u16);
    put16(&mut file, 56, segments.len() as u16);
    for (index, segment) in segments.iter().enumerate() {
        let header = PH + index * PH_SIZE;
        put32(&mut file, header, segment.kind);
        put32(&mut file, header + 4, segment.flags);
        put64(&mut file, header + 8, segment.offset);
        put64(&mut file, header + 16, segment.address);
        put64(&mut file, header + 32, segment.contents.len() as u64);
        put64(&mut file, header + 40, segment.memory_len);
        put64(&mut file, header + 48, segment.alignment);
        let offset = segment.offset as usize;
        file[offset..offset + segment.contents.len()].copy_from_slice(segment.contents);
    }
    file
}

fn parse(file: &[u8]) -> Result<ElfImage<'_, 8>, ElfError> {
    ElfImage::parse(file, LIMITS)
}

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn static_image_has_borrowed_pages_with_exact_permissions() {
    let read_only = Segment {
        flags: 4,
        offset: 0x2000,
        address: DATA,
        contents: b"read-only",
        memory_len: 4096,
        ..text()
    };
    let writable = Segment {
        flags: 6,
        offset: 0x3000,
        address: DATA + 0x2000,
        contents: &[42],
        memory_len: 8192,
        ..text()
    };
    let file = elf(&[text(), read_only, writable], TEXT + 1);
    let image = parse(&file).unwrap();
    assert_eq!(image.entry(), TEXT + 1);
    assert_eq!(image.page_count(), 4);
    assert_eq!(image.segments().len(), 3);
    assert_eq!(
        image
            .pages()
            .map(|page| (page.address(), page.permissions()))
            .collect::<Vec<_>>(),
        [
            (TEXT, Permissions::ReadExecute),
            (DATA, Permissions::ReadOnly),
            (DATA + 0x2000, Permissions::ReadWrite),
            (DATA + 0x3000, Permissions::ReadWrite),
        ]
    );
    let segment = image.segments().nth(2).unwrap();
    assert_eq!(segment.address(), DATA + 0x2000);
    assert_eq!(segment.memory_len(), 8192);
    assert_eq!(segment.permissions(), Permissions::ReadWrite);
    assert_eq!(segment.page_start(), DATA + 0x2000);
    assert_eq!(segment.page_end(), DATA + 0x4000);
}

#[test]
fn every_truncated_prefix_is_rejected_without_panicking() {
    let file = elf(&[text()], TEXT);
    for length in 0..file.len() {
        assert!(parse(&file[..length]).is_err(), "accepted prefix {length}");
    }
}

#[test]
fn invalid_identity_or_header_size_is_rejected() {
    for (offset, value) in [(0, 0), (3, b'X'), (6, 0), (52, 63)] {
        let mut file = elf(&[text()], TEXT);
        file[offset] = value;
        assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidHeader);
    }
}

#[test]
fn only_native_little_endian_amd64_static_images_are_supported() {
    for (offset, value) in [
        (4, 1),
        (5, 2),
        (7, 3),
        (8, 1),
        (16, 3),
        (18, 3),
        (20, 0),
        (48, 1),
    ] {
        let mut file = elf(&[text()], TEXT);
        file[offset] = value;
        assert_eq!(parse(&file).unwrap_err(), ElfError::UnsupportedFormat);
    }
}

#[test]
fn program_header_count_and_size_are_bounded() {
    for count in [0, 65, u16::MAX] {
        let mut file = elf(&[text()], TEXT);
        put16(&mut file, 56, count);
        assert_eq!(parse(&file).unwrap_err(), ElfError::ProgramHeaders);
    }
    for size in [0, 55, 57, u16::MAX] {
        let mut file = elf(&[text()], TEXT);
        put16(&mut file, 54, size);
        assert_eq!(parse(&file).unwrap_err(), ElfError::ProgramHeaders);
    }
}

#[test]
fn program_header_arithmetic_and_table_bounds_are_checked() {
    for offset in [0, 56, 65, 0x2000, u64::MAX - 7, u64::MAX] {
        let mut file = elf(&[text()], TEXT);
        put64(&mut file, 32, offset);
        assert_eq!(parse(&file).unwrap_err(), ElfError::ProgramHeaders);
    }
    let mut file = elf(&[text()], TEXT);
    put16(&mut file, 56, 2);
    file.truncate(PH + PH_SIZE);
    assert_eq!(parse(&file).unwrap_err(), ElfError::ProgramHeaders);
}

#[test]
fn executable_plans_do_not_depend_on_section_tables() {
    let mut file = elf(&[text()], TEXT);
    put64(&mut file, 40, u64::MAX);
    put16(&mut file, 58, u16::MAX);
    put16(&mut file, 60, u16::MAX);
    put16(&mut file, 62, u16::MAX);
    assert_eq!(parse(&file).unwrap().entry(), TEXT);
}

#[test]
fn segment_capacity_and_page_budget_are_independent() {
    let data = Segment {
        offset: 0x2000,
        address: DATA,
        flags: 6,
        ..text()
    };
    let file = elf(&[text(), data], TEXT);
    assert_eq!(
        ElfImage::<1>::parse(&file, LIMITS).unwrap_err(),
        ElfError::SegmentCapacity
    );
    assert_eq!(
        ElfImage::<0>::parse(&file, LIMITS).unwrap_err(),
        ElfError::SegmentCapacity
    );
    assert_eq!(
        ElfImage::<8>::parse(&file, LoadLimits { max_pages: 1 }).unwrap_err(),
        ElfError::PageLimit
    );
    assert_eq!(
        ElfImage::<8>::parse(&file, LoadLimits { max_pages: 0 }).unwrap_err(),
        ElfError::PageLimit
    );
    assert!(ElfImage::<2>::parse(&file, LoadLimits { max_pages: 2 }).is_ok());
}

#[test]
fn huge_valid_user_ranges_are_rejected_by_the_page_budget() {
    let mut file = elf(&[text()], TEXT);
    put64(&mut file, PH + 40, USER_END - TEXT);
    assert_eq!(parse(&file).unwrap_err(), ElfError::PageLimit);
}

#[test]
fn segment_file_size_must_not_exceed_memory_size() {
    let mut file = elf(&[text()], TEXT);
    put64(&mut file, PH + 40, 3);
    assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidSegment);
}

#[test]
fn file_ranges_reject_overflow_and_out_of_bounds_bytes() {
    for (offset, length) in [(u64::MAX - 3, 8), (0x1002, 4), (u64::MAX, 0)] {
        let mut file = elf(&[text()], TEXT);
        put64(&mut file, PH + 8, offset);
        put64(&mut file, PH + 32, length);
        put64(&mut file, PH + 40, length.max(1));
        assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidSegment);
    }
}

#[test]
fn load_alignment_checks_power_two_and_both_congruence_rules() {
    for (address, alignment) in [(TEXT, 3), (TEXT, 8192), (TEXT + 1, 0), (TEXT + 1, 1)] {
        let mut file = elf(&[text()], address);
        put64(&mut file, PH + 16, address);
        put64(&mut file, PH + 48, alignment);
        assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidAlignment);
    }
    for alignment in [0, 1, 4096] {
        let file = elf(
            &[Segment {
                alignment,
                ..text()
            }],
            TEXT,
        );
        assert!(parse(&file).is_ok());
    }
}

#[test]
fn load_addresses_exclude_null_kernel_noncanonical_and_wrapping_ranges() {
    for (address, memory_len) in [
        (0, 4),
        (USER_END, 4),
        (0xffff_8000_0000_0000, 4),
        (u64::MAX - 4095, 8192),
        (USER_END - 4096, 4097),
    ] {
        let mut file = elf(&[text()], address);
        put64(&mut file, PH + 16, address);
        put64(&mut file, PH + 40, memory_len);
        assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidAddress);
    }
    let edge = Segment {
        address: USER_END - 4096,
        memory_len: 4096,
        ..text()
    };
    let file = elf(&[edge], edge.address);
    assert_eq!(
        parse(&file).unwrap().pages().next().unwrap().address(),
        edge.address
    );
}

#[test]
fn writable_executable_and_unrepresentable_permissions_are_rejected() {
    for flags in [0, 1, 2, 3, 7, 0x1000_0004] {
        let file = elf(&[Segment { flags, ..text() }], TEXT);
        assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidPermissions);
    }
}

#[test]
fn byte_disjoint_segments_must_not_share_a_page() {
    for address in [TEXT, TEXT + 2, TEXT + 0x800] {
        let other = Segment {
            offset: 0x2000 + address % 4096,
            address,
            flags: 4,
            ..text()
        };
        let file = elf(&[text(), other], TEXT);
        assert_eq!(parse(&file).unwrap_err(), ElfError::OverlappingPages);
    }
}

#[test]
fn entry_must_be_inside_file_backed_executable_bytes() {
    let code = Segment {
        offset: 0x1010,
        address: TEXT + 0x10,
        memory_len: 0x2000,
        ..text()
    };
    let data = Segment {
        address: DATA,
        offset: 0x2000,
        flags: 6,
        ..text()
    };
    for entry in [0, TEXT, code.address + 4, TEXT + 0x1000, DATA, USER_END] {
        let file = elf(&[code, data], entry);
        assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidEntry);
    }
    let file = elf(&[code, data], code.address + 3);
    assert!(parse(&file).is_ok());
}

#[test]
fn dynamic_and_unknown_program_headers_are_rejected() {
    for kind in [2, 3, 5, 0x6474_e552, 0x6474_e553, u32::MAX] {
        let extra = Segment {
            kind,
            offset: 0,
            contents: &[],
            ..text()
        };
        let file = elf(&[text(), extra], TEXT);
        assert_eq!(parse(&file).unwrap_err(), ElfError::UnsupportedSegment);
    }
}

#[test]
fn null_and_note_headers_do_not_add_mappings() {
    let mut extras = [text(); 3];
    for (index, kind) in [0, 4].into_iter().enumerate() {
        extras[index + 1] = Segment {
            kind,
            offset: 0,
            contents: &[],
            ..text()
        };
    }
    let file = elf(&extras, TEXT);
    assert_eq!(parse(&file).unwrap().page_count(), 1);
    let mut bad_note = file;
    put64(&mut bad_note, PH + 2 * PH_SIZE + 8, u64::MAX);
    put64(&mut bad_note, PH + 2 * PH_SIZE + 32, 1);
    assert_eq!(parse(&bad_note).unwrap_err(), ElfError::InvalidSegment);
}

const HEADERS: u64 = 0x30_0000;

fn metadata_elf(extra: &[Segment<'_>]) -> Vec<u8> {
    let mut segments = vec![
        Segment {
            kind: 6,
            flags: 4,
            offset: PH as u64,
            address: HEADERS + PH as u64,
            contents: &[],
            memory_len: 0,
            alignment: 8,
        },
        Segment {
            kind: 1,
            flags: 4,
            offset: 0,
            address: HEADERS,
            contents: &[],
            memory_len: 4096,
            alignment: 4096,
        },
        text(),
    ];
    segments.extend_from_slice(extra);
    let mut file = elf(&segments, TEXT);
    let table_len = (segments.len() * PH_SIZE) as u64;
    put64(&mut file, PH + 32, table_len);
    put64(&mut file, PH + 40, table_len);
    put64(&mut file, PH + PH_SIZE + 32, PH as u64 + table_len);
    file
}

fn tls_segments() -> [Segment<'static>; 2] {
    let template = Segment {
        offset: 0x2000,
        address: DATA,
        flags: 6,
        contents: &[7, 8, 9, 10],
        memory_len: 4,
        alignment: 4096,
        ..text()
    };
    [
        template,
        Segment {
            kind: 7,
            flags: 4,
            memory_len: 128,
            alignment: 64,
            ..template
        },
    ]
}

#[test]
fn mapped_metadata_reports_real_read_only_addresses() {
    let file = metadata_elf(&[]);
    let image = parse(&file).unwrap();
    assert_eq!(image.elf_header_address(), Some(HEADERS));
    let headers = image.program_headers().unwrap();
    assert_eq!(headers.address, HEADERS + PH as u64);
    assert_eq!(headers.file_offset, PH as u64);
    assert_eq!(headers.entry_size, PH_SIZE as u64);
    assert_eq!(headers.count, 3);
    assert_eq!(image.page_count(), 2);
    assert_eq!(image.segments().next().unwrap().file_offset(), 0);
    let mut page = [0; 4096];
    image.pages().next().unwrap().initialize(&mut page);
    assert_eq!(&page[..PH + PH_SIZE * 3], &file[..PH + PH_SIZE * 3]);
}

#[test]
fn separately_mapped_metadata_retains_its_actual_file_offset_and_address() {
    let table_len = (PH_SIZE * 4) as u64;
    let table = Segment {
        offset: PH as u64,
        address: DATA + PH as u64,
        flags: 4,
        contents: &[],
        memory_len: table_len,
        alignment: 8,
        ..text()
    };
    let mut file = metadata_elf(&[table]);
    put64(&mut file, PH + 16, table.address);
    put64(&mut file, PH + PH_SIZE + 32, PH as u64);
    put64(&mut file, PH + PH_SIZE * 3 + 32, table_len);
    let image = parse(&file).unwrap();
    let header = image.elf_header_address().unwrap();
    let headers = image.program_headers().unwrap();
    assert_eq!(header, HEADERS);
    assert_eq!(headers.file_offset, PH as u64);
    assert_eq!(headers.address, table.address);
    assert_eq!(headers.entry_size, PH_SIZE as u64);
    assert_eq!(headers.count, 4);
    assert_ne!(
        header.checked_add(headers.file_offset),
        Some(headers.address)
    );
}

#[test]
fn legacy_images_do_not_invent_mapped_metadata() {
    let file = elf(&[text()], TEXT);
    let image = parse(&file).unwrap();
    assert_eq!(image.elf_header_address(), None);
    assert_eq!(image.program_headers(), None);
    assert!(image.tls().is_none());

    let mut file = metadata_elf(&[]);
    put32(&mut file, PH, 0);
    put64(&mut file, PH + PH_SIZE + 32, PH as u64);
    let image = parse(&file).unwrap();
    assert_eq!(image.elf_header_address(), Some(HEADERS));
    assert_eq!(image.program_headers(), None);
}

#[test]
fn mutable_or_ambiguous_metadata_is_not_exposed() {
    let mut writable = metadata_elf(&[]);
    put32(&mut writable, PH, 0);
    put32(&mut writable, PH + PH_SIZE + 4, 6);
    let image = parse(&writable).unwrap();
    assert_eq!(image.elf_header_address(), None);
    assert_eq!(image.program_headers(), None);

    let alias = Segment {
        offset: 0,
        address: HEADERS + 0x10_000,
        flags: 4,
        contents: &[],
        memory_len: 4096,
        ..text()
    };
    let mut file = metadata_elf(&[alias]);
    put32(&mut file, PH, 0);
    put64(&mut file, PH + PH_SIZE * 3 + 32, (PH + PH_SIZE * 4) as u64);
    let image = parse(&file).unwrap();
    assert_eq!(image.elf_header_address(), None);
    assert_eq!(image.program_headers(), None);
}

#[test]
fn phdr_description_must_match_the_actual_mapped_table() {
    let original = metadata_elf(&[]);
    for (offset, value) in [
        (8, 0),
        (16, HEADERS + PH as u64 + 8),
        (32, (PH_SIZE * 3 - 1) as u64),
        (40, (PH_SIZE * 3 + 1) as u64),
        (48, 3),
    ] {
        let mut file = original.clone();
        put64(&mut file, PH + offset, value);
        assert_eq!(parse(&file).unwrap_err(), ElfError::ProgramHeaders);
    }
    let mut file = original.clone();
    put32(&mut file, PH + PH_SIZE + 4, 6);
    assert_eq!(parse(&file).unwrap_err(), ElfError::ProgramHeaders);
    let mut file = original.clone();
    put64(&mut file, PH + PH_SIZE + 32, PH as u64);
    assert_eq!(parse(&file).unwrap_err(), ElfError::ProgramHeaders);
    let mut file = original;
    file[PH..PH + PH_SIZE * 3].rotate_left(PH_SIZE);
    assert_eq!(parse(&file).unwrap_err(), ElfError::ProgramHeaders);
}

#[test]
fn duplicate_phdr_descriptions_are_rejected() {
    let mut file = metadata_elf(&[text()]);
    file.copy_within(PH..PH + PH_SIZE, PH + PH_SIZE * 3);
    assert_eq!(parse(&file).unwrap_err(), ElfError::ProgramHeaders);
}

#[test]
fn static_tls_borrows_initialization_and_keeps_bss_separate() {
    let file = metadata_elf(&tls_segments());
    let image = parse(&file).unwrap();
    let tls = image.tls().unwrap();
    assert_eq!(tls.address(), DATA);
    assert_eq!(tls.contents(), [7, 8, 9, 10]);
    assert_eq!(tls.memory_len(), 128);
    assert_eq!(tls.alignment(), 64);
    assert_eq!(image.page_count(), 3);
    // The initialized template is mapped; the per-thread zero tail does not
    // consume more image pages or need to fit in PT_LOAD.p_memsz.
    assert_eq!(image.segments().last().unwrap().memory_len(), 4);
}

#[test]
fn static_tls_can_be_zero_only_or_empty() {
    for (address, memory_len) in [(DATA, 128), (0, 0)] {
        let tls = Segment {
            kind: 7,
            flags: 4,
            offset: 0,
            address,
            contents: &[],
            memory_len,
            alignment: 0,
        };
        let file = metadata_elf(&[tls]);
        let image = parse(&file).unwrap();
        let tls = image.tls().unwrap();
        assert!(tls.contents().is_empty());
        assert_eq!(tls.memory_len(), memory_len);
        assert_eq!(tls.alignment(), 1);
        assert_eq!(image.page_count(), 2);
    }
}

#[test]
fn tls_requires_one_exact_file_backing_translation() {
    let original = metadata_elf(&tls_segments());
    let tls_header = PH + PH_SIZE * 4;
    let mut mismatch = original.clone();
    put64(&mut mismatch, tls_header + 16, DATA + 64);
    assert_eq!(parse(&mismatch).unwrap_err(), ElfError::InvalidTls);
    let mut mismatch = original;
    put64(&mut mismatch, tls_header + 8, 0x2001);
    put64(&mut mismatch, tls_header + 32, 3);
    put64(&mut mismatch, tls_header + 48, 1);
    assert_eq!(parse(&mismatch).unwrap_err(), ElfError::InvalidTls);

    let mut segments = tls_segments().to_vec();
    segments.push(Segment {
        address: DATA + 4096,
        ..segments[0]
    });
    let file = metadata_elf(&segments);
    assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidTls);

    let file = metadata_elf(&[tls_segments()[1]]);
    assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidTls);
}

#[test]
fn malformed_tls_descriptions_are_rejected() {
    let original = metadata_elf(&tls_segments());
    let tls_header = PH + PH_SIZE * 4;
    for flags in [0, 5, 6, 7, 0x1000_0004] {
        let mut file = original.clone();
        put32(&mut file, tls_header + 4, flags);
        assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidPermissions);
    }
    let mut file = original.clone();
    put64(&mut file, tls_header + 40, 3);
    assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidTls);
    for (address, size) in [(0, 128), (USER_END, 128), (u64::MAX - 63, 128)] {
        let mut file = original.clone();
        put64(&mut file, tls_header + 16, address);
        put64(&mut file, tls_header + 40, size);
        put64(&mut file, tls_header + 48, 1);
        assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidAddress);
    }
    for (address, alignment) in [(DATA, 3), (DATA + 1, 64)] {
        let mut file = original.clone();
        put64(&mut file, tls_header + 16, address);
        put64(&mut file, tls_header + 48, alignment);
        assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidAlignment);
    }
    let mut file = original;
    put64(&mut file, tls_header + 8, u64::MAX - 1);
    put64(&mut file, tls_header + 48, 1);
    assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidSegment);

    let mut duplicate = tls_segments().to_vec();
    duplicate.push(duplicate[1]);
    let file = metadata_elf(&duplicate);
    assert_eq!(parse(&file).unwrap_err(), ElfError::InvalidTls);
}

#[test]
fn hostile_tls_fields_do_not_panic_or_add_image_pages() {
    let original = metadata_elf(&tls_segments());
    let tls_header = PH + PH_SIZE * 4;
    let mut random = 0x0123_4567_89ab_cdefu64;
    for _ in 0..1024 {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        for field in [8, 16, 32, 40, 48] {
            let mut file = original.clone();
            put64(&mut file, tls_header + field, random);
            if let Ok(image) = parse(&file) {
                assert_eq!(image.page_count(), 3);
                assert!(image.tls().is_some());
            }
        }
    }
}

#[test]
fn executable_stack_requests_are_rejected() {
    for flags in [0, 4, 6] {
        let stack = Segment {
            kind: 0x6474_e551,
            flags,
            offset: 0,
            contents: &[],
            ..text()
        };
        assert!(parse(&elf(&[text(), stack], TEXT)).is_ok());
    }
    for flags in [1, 7, 0x1000_0006] {
        let stack = Segment {
            kind: 0x6474_e551,
            flags,
            offset: 0,
            contents: &[],
            ..text()
        };
        assert_eq!(
            parse(&elf(&[text(), stack], TEXT)).unwrap_err(),
            ElfError::InvalidPermissions
        );
    }
}

#[test]
fn zero_length_loads_add_no_pages_and_cannot_supply_an_entry() {
    let empty = Segment {
        contents: &[],
        memory_len: 0,
        ..text()
    };
    assert_eq!(
        parse(&elf(&[empty], TEXT)).unwrap_err(),
        ElfError::NoSegments
    );
    let file = elf(&[text(), empty], TEXT);
    assert_eq!(parse(&file).unwrap().page_count(), 1);
}

#[test]
fn bss_only_data_pages_are_zeroed_without_reading_file_bytes() {
    let bss = Segment {
        offset: 0x2000,
        address: DATA,
        contents: &[],
        memory_len: 8192,
        flags: 6,
        ..text()
    };
    let file = elf(&[text(), bss], TEXT);
    let image = parse(&file).unwrap();
    let mut buffer = [0xa5; 4096];
    for page in image.pages().skip(1) {
        page.initialize(&mut buffer);
        assert_eq!(buffer, [0; 4096]);
    }
}

#[test]
fn partial_pages_and_bss_match_an_independent_byte_oracle() {
    let payload: Vec<u8> = (0..6000).map(|index| (index % 251 + 1) as u8).collect();
    for leading in [0, 1, 13, 4095] {
        for file_len in [0, 1, 257, 4096, 6000] {
            for bss_len in [0, 1, 4096, 5000] {
                if file_len + bss_len == 0 {
                    continue;
                }
                let data = Segment {
                    offset: 0x2000 + leading,
                    address: DATA + leading,
                    contents: &payload[..file_len],
                    memory_len: (file_len + bss_len) as u64,
                    flags: 6,
                    ..text()
                };
                let file = elf(&[text(), data], TEXT);
                let image = parse(&file).unwrap();
                let mut buffer = [0xa5; 4096];
                let mut pages = 1;
                for page in image.pages().skip(1) {
                    page.initialize(&mut buffer);
                    for (offset, byte) in buffer.iter().enumerate() {
                        let address = page.address() + offset as u64;
                        let expected = address
                            .checked_sub(data.address)
                            .and_then(|offset| {
                                payload
                                    .get(offset as usize)
                                    .filter(|_| offset < file_len as u64)
                            })
                            .copied()
                            .unwrap_or(0);
                        assert_eq!(
                            *byte, expected,
                            "at {address:#x}, lead={leading}, file={file_len}, bss={bss_len}"
                        );
                    }
                    pages += 1;
                }
                assert_eq!(image.page_count(), pages);
            }
        }
    }
}

#[test]
fn hostile_segment_fields_never_panic_or_exceed_the_plan_budget() {
    let original = elf(&[text()], TEXT);
    let mut random = 0x1234_5678_9abc_def0u64;
    for _ in 0..4096 {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        for field in [8, 16, 32, 40, 48] {
            let mut file = original.clone();
            put64(&mut file, PH + field, random);
            if let Ok(image) = parse(&file) {
                assert!(image.page_count() <= LIMITS.max_pages);
                let mut buffer = [0xa5; 4096];
                for page in image.pages() {
                    page.initialize(&mut buffer);
                }
            }
        }
    }
}
