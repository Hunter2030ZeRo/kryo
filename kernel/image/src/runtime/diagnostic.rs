use core::fmt::{self, Write};

pub(super) const LIMIT: usize = 256;
const PAGE_BYTES: u64 = 4096;
const USER_END: u64 = 1 << 47;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SnapshotError<E> {
    InvalidRange,
    Read(E),
}

pub(super) fn snapshot<E>(
    address: u64,
    bytes: u64,
    output: &mut [u8; LIMIT],
    mut read: impl FnMut(u64, usize, &mut [u8]) -> Result<(), E>,
) -> Result<usize, SnapshotError<E>> {
    let length = usize::try_from(bytes).map_err(|_| SnapshotError::InvalidRange)?;
    if length > LIMIT {
        return Err(SnapshotError::InvalidRange);
    }
    if length == 0 {
        return Ok(0);
    }
    if address == 0 || address.checked_add(bytes).is_none_or(|end| end > USER_END) {
        return Err(SnapshotError::InvalidRange);
    }

    let mut copied = 0;
    while copied < length {
        let current = address + copied as u64;
        let page = current & !(PAGE_BYTES - 1);
        let offset = (current - page) as usize;
        let count = (length - copied).min(PAGE_BYTES as usize - offset);
        read(page, offset, &mut output[copied..copied + count]).map_err(SnapshotError::Read)?;
        copied += count;
    }
    Ok(length)
}

pub(super) struct Escaped<'a>(pub(super) &'a [u8]);

impl fmt::Display for Escaped<'_> {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &byte in self.0 {
            match byte {
                b'\\' => output.write_str("\\\\")?,
                0x20..=0x7e => output.write_char(byte as char)?,
                _ => write!(output, "\\x{byte:02x}")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_snapshot_does_not_read_the_pointer() {
        let mut output = [0; LIMIT];
        assert_eq!(
            snapshot(u64::MAX, 0, &mut output, |_, _, _| -> Result<(), ()> {
                panic!("Empty diagnostic read user memory")
            }),
            Ok(0)
        );
    }

    #[test]
    fn malformed_ranges_do_not_read_memory() {
        for (address, bytes) in [
            (4096, LIMIT as u64 + 1),
            (0, 1),
            (u64::MAX, 2),
            (USER_END, 1),
            (USER_END - 1, 2),
        ] {
            assert_eq!(
                snapshot(
                    address,
                    bytes,
                    &mut [0; LIMIT],
                    |_, _, _| -> Result<(), ()> { panic!("Invalid diagnostic read user memory") }
                ),
                Err(SnapshotError::InvalidRange)
            );
        }
    }

    #[test]
    fn unaligned_snapshot_reads_at_most_two_pages() {
        let mut output = [0; LIMIT];
        let mut reads = std::vec::Vec::new();
        let length = snapshot(8190, LIMIT as u64, &mut output, |page, offset, bytes| {
            reads.push((page, offset, bytes.len()));
            bytes.fill(if page == 4096 { b'A' } else { b'B' });
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(length, LIMIT);
        assert_eq!(reads, [(4096, 4094, 2), (8192, 0, LIMIT - 2)]);
        assert_eq!(&output[..2], b"AA");
        assert!(output[2..].iter().all(|byte| *byte == b'B'));
    }

    #[test]
    fn exact_page_end_needs_no_second_read() {
        let mut reads = 0;
        assert_eq!(
            snapshot(
                USER_END - LIMIT as u64,
                LIMIT as u64,
                &mut [0; LIMIT],
                |page, offset, bytes| {
                    reads += 1;
                    assert_eq!(page, USER_END - PAGE_BYTES);
                    assert_eq!(offset, PAGE_BYTES as usize - LIMIT);
                    assert_eq!(bytes.len(), LIMIT);
                    Ok::<_, ()>(())
                }
            ),
            Ok(LIMIT)
        );
        assert_eq!(reads, 1);
    }

    #[test]
    fn unreadable_second_page_fails_the_whole_snapshot() {
        let mut reads = 0;
        let mut output = [0; LIMIT];
        assert_eq!(
            snapshot(8191, 2, &mut output, |page, _, bytes| {
                reads += 1;
                if page == 8192 {
                    Err("unmapped")
                } else {
                    bytes.fill(b'X');
                    Ok(())
                }
            }),
            Err(SnapshotError::Read("unmapped"))
        );
        assert_eq!(reads, 2);
    }

    #[test]
    fn text_cannot_inject_control_bytes_or_marker_lines() {
        assert_eq!(
            std::format!("{}", Escaped(b"\nPANIC\r\t\0\x1b\\\x7f\x80\xff")),
            "\\x0aPANIC\\x0d\\x09\\x00\\x1b\\\\\\x7f\\x80\\xff"
        );
        assert_eq!(
            std::format!("{}", Escaped(b"printable ~ text")),
            "printable ~ text"
        );
    }

    #[test]
    fn maximum_escaped_output_is_bounded() {
        let escaped = std::format!("{}", Escaped(&[0; LIMIT]));
        assert_eq!(escaped.len(), LIMIT * 4);
        assert!(escaped.bytes().all(|byte| byte.is_ascii_graphic()));
    }
}
