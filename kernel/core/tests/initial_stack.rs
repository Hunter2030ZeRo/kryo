use kryo_core::exec::{AuxEntry, StackError, build_initial_stack};

const BASE: u64 = 0x80_0000;
const USER_END: u64 = 0x8000_0000_0000;

fn word(bytes: &[u8], base: u64, address: u64) -> u64 {
    let offset = usize::try_from(address.checked_sub(base).unwrap()).unwrap();
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn string(bytes: &[u8], base: u64, address: u64) -> &[u8] {
    let offset = usize::try_from(address.checked_sub(base).unwrap()).unwrap();
    let contents = &bytes[offset..];
    let end = contents.iter().position(|byte| *byte == 0).unwrap();
    &contents[..end]
}

#[test]
fn stack_vectors_preserve_order_and_owned_strings() {
    let mut output = [0xa5; 512];
    let aux = [
        AuxEntry {
            kind: 6,
            value: 4096,
        },
        AuxEntry {
            kind: 9,
            value: 0x40_0123,
        },
    ];
    let argv: &[&[u8]] = &[b"/bin/probe", b"", b"argument"];
    let envp: &[&[u8]] = &[b"NAME=value", b"RAW=\xff"];
    let pointer = build_initial_stack(&mut output, BASE, argv, envp, &aux).unwrap();
    assert_eq!(pointer % 16, 0);
    let mut cursor = pointer;
    assert_eq!(word(&output, BASE, cursor), 3);
    cursor += 8;
    for expected in argv {
        let address = word(&output, BASE, cursor);
        assert!((pointer..BASE + output.len() as u64).contains(&address));
        assert_eq!(string(&output, BASE, address), *expected);
        cursor += 8;
    }
    assert_eq!(word(&output, BASE, cursor), 0);
    cursor += 8;
    for expected in envp {
        assert_eq!(
            string(&output, BASE, word(&output, BASE, cursor)),
            *expected
        );
        cursor += 8;
    }
    assert_eq!(word(&output, BASE, cursor), 0);
    cursor += 8;
    for expected in aux {
        assert_eq!(word(&output, BASE, cursor), expected.kind);
        assert_eq!(word(&output, BASE, cursor + 8), expected.value);
        cursor += 16;
    }
    assert_eq!(word(&output, BASE, cursor), 0);
    assert_eq!(word(&output, BASE, cursor + 8), 0);
    let first_string = word(&output, BASE, pointer + 8);
    assert!(cursor + 16 <= first_string);
    assert!(
        output[..(pointer - BASE) as usize]
            .iter()
            .all(|byte| *byte == 0)
    );
    assert!(
        output[(cursor + 16 - BASE) as usize..(first_string - BASE) as usize]
            .iter()
            .all(|byte| *byte == 0)
    );
}

#[test]
fn empty_vectors_and_unaligned_storage_still_produce_aligned_rsp() {
    for base_offset in 0..16 {
        for length in 40..80 {
            let base = BASE + base_offset;
            let mut output = vec![0xa5; length];
            match build_initial_stack(&mut output, base, &[], &[], &[]) {
                Ok(pointer) => {
                    assert_eq!(pointer % 16, 0);
                    assert!(pointer >= base);
                    assert!(pointer + 40 <= base + length as u64);
                    assert!(output.iter().all(|byte| *byte == 0));
                }
                Err(error) => {
                    assert_eq!(error, StackError::InsufficientSpace);
                    assert!(output.iter().all(|byte| *byte == 0xa5));
                    assert!((base + length as u64 - 40) & !15 < base);
                }
            }
        }
    }
}

#[test]
fn strings_are_copied_and_not_borrowed_from_the_argument_storage() {
    let mut output = [0; 128];
    let mut argument = *b"hello";
    let pointer = build_initial_stack(&mut output, BASE, &[&argument], &[], &[]).unwrap();
    argument.fill(b'x');
    assert_eq!(
        string(&output, BASE, word(&output, BASE, pointer + 8)),
        b"hello"
    );
}

#[test]
fn insufficient_capacity_preserves_all_output_bytes() {
    for length in 0..96 {
        let mut output = vec![0xa5; length];
        let before = output.clone();
        let result = build_initial_stack(
            &mut output,
            BASE,
            &[b"a long enough argument to fill a small stack"],
            &[b"KEY=value"],
            &[AuxEntry {
                kind: 6,
                value: 4096,
            }],
        );
        assert_eq!(result, Err(StackError::InsufficientSpace));
        assert_eq!(output, before);
    }
}

#[test]
fn embedded_nul_and_invalid_auxiliary_entries_do_not_change_output() {
    let mut output = [0xa5; 128];
    for (argv, envp) in [
        (&[&b"a\0b"[..]][..], &[][..]),
        (&[][..], &[&b"A=\0b"[..]][..]),
    ] {
        assert_eq!(
            build_initial_stack(&mut output, BASE, argv, envp, &[]),
            Err(StackError::EmbeddedNul)
        );
        assert_eq!(output, [0xa5; 128]);
    }
    for aux in [
        &[AuxEntry { kind: 0, value: 0 }][..],
        &[
            AuxEntry {
                kind: 6,
                value: 4096,
            },
            AuxEntry {
                kind: 6,
                value: 8192,
            },
        ][..],
    ] {
        assert_eq!(
            build_initial_stack(&mut output, BASE, &[], &[], aux),
            Err(StackError::InvalidAuxiliary)
        );
        assert_eq!(output, [0xa5; 128]);
    }
}

#[test]
fn invalid_and_overflowing_user_ranges_preserve_output() {
    let mut output = [0xa5; 128];
    for base in [0, 4095, USER_END, USER_END - 127, u64::MAX - 63] {
        assert_eq!(
            build_initial_stack(&mut output, base, &[], &[], &[]),
            Err(StackError::InvalidAddress)
        );
        assert_eq!(output, [0xa5; 128]);
    }
    let pointer = build_initial_stack(&mut output, USER_END - 128, &[], &[], &[]).unwrap();
    assert_eq!(pointer % 16, 0);
    assert!(pointer < USER_END);
}

#[test]
fn strings_and_vectors_fit_exactly_without_overlap() {
    // One argv pointer adds eight bytes; its seven-byte string plus NUL leaves
    // exactly 48 metadata bytes and eight string bytes in this mapping.
    let mut output = [0xa5; 56];
    let pointer = build_initial_stack(&mut output, BASE, &[b"1234567"], &[], &[]).unwrap();
    assert_eq!(pointer, BASE);
    assert_eq!(word(&output, BASE, pointer), 1);
    assert_eq!(word(&output, BASE, pointer + 8), BASE + 48);
    assert_eq!(&output[48..], b"1234567\0");
}
