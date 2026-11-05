//! Experimental file RPC. The receiving gate authenticates the namespace or
//! open description; request IDs only correlate replies.

pub const VERSION: u64 = 1;
pub const PAGE_SIZE: u64 = 4096;
pub const NAMESPACE_ROLE: u64 = 2;
pub const NAMESPACE_LABEL: u64 = 1;

pub const READ_ONLY: u32 = 0;
pub const WRITE_ONLY: u32 = 1;
pub const READ_WRITE: u32 = 2;
pub const CREATE: u32 = 0x10;
pub const EXCLUSIVE: u32 = 0x20;
pub const TRUNCATE: u32 = 0x40;
pub const APPEND: u32 = 0x80;
pub const OPEN_MASK: u32 = 3 | CREATE | EXCLUSIVE | TRUNCATE | APPEND;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Operation {
    Open = 1,
    Read = 2,
    Write = 3,
    Seek = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Status {
    Ok = 0,
    Malformed = 1,
    Denied = 2,
    Range = 3,
    Unsupported = 4,
    NotFound = 5,
    Exists = 6,
    NoSpace = 7,
    Io = 8,
    Unavailable = 9,
    Overflow = 10,
    TooManyOpen = 11,
    BadFile = 12,
}

pub const fn header(operation: Operation) -> u64 {
    (VERSION << 32) | operation as u64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenOptions {
    pub flags: u32,
    pub mode: u32,
}

impl OpenOptions {
    pub fn decode(word: u64) -> Result<Self, Status> {
        let flags = word as u32;
        let mode = (word >> 32) as u32;
        if flags & !OPEN_MASK != 0
            || flags & 3 == 3
            || mode & !0o7777 != 0
            || (flags & CREATE == 0 && (mode != 0 || flags & EXCLUSIVE != 0))
            || (flags & TRUNCATE != 0 && flags & 3 == READ_ONLY)
        {
            return Err(Status::Malformed);
        }
        Ok(Self { flags, mode })
    }

    pub const fn readable(self) -> bool {
        self.flags & 3 != WRITE_ONLY
    }

    pub const fn writable(self) -> bool {
        self.flags & 3 != READ_ONLY
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub operation: Operation,
    pub id: u64,
    pub argument: u64,
    pub length: u64,
}

impl Request {
    pub fn decode(message: [u64; 4], namespace: bool, capabilities: usize) -> Result<Self, Status> {
        if message[0] >> 32 != VERSION || message[1] == 0 {
            return Err(Status::Malformed);
        }
        let operation = match message[0] as u32 {
            1 => Operation::Open,
            2 => Operation::Read,
            3 => Operation::Write,
            4 => Operation::Seek,
            _ => return Err(Status::Malformed),
        };
        if namespace != (operation == Operation::Open) {
            return Err(Status::Denied);
        }
        let expected = usize::from(operation != Operation::Seek);
        if capabilities != expected {
            return Err(Status::Malformed);
        }
        match operation {
            Operation::Open => {
                OpenOptions::decode(message[2])?;
                if message[3] == 0 || message[3] > PAGE_SIZE {
                    return Err(Status::Range);
                }
            }
            Operation::Read | Operation::Write => {
                if message[2]
                    .checked_add(message[3])
                    .is_none_or(|end| end > PAGE_SIZE)
                {
                    return Err(Status::Range);
                }
            }
            Operation::Seek => {
                if message[3] > 2 {
                    return Err(Status::Malformed);
                }
            }
        }
        Ok(Self {
            operation,
            id: message[1],
            argument: message[2],
            length: message[3],
        })
    }
}

pub const fn response(id: u64, status: Status, value: u64) -> [u64; 4] {
    [id, status as u64, value, 0]
}

/// The signed seek result must fit the public off_t without wrapping.
pub fn seek_position(current: u64, size: u64, offset: i64, whence: u64) -> Result<u64, Status> {
    let base = match whence {
        0 => 0,
        1 => current,
        2 => size,
        _ => return Err(Status::Malformed),
    };
    let result = i128::from(base) + i128::from(offset);
    if result < 0 {
        Err(Status::Range)
    } else if result > i128::from(i64::MAX) {
        Err(Status::Overflow)
    } else {
        Ok(result as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_and_file_authority_come_from_the_gate() {
        assert_eq!(
            Request::decode([header(Operation::Open), 1, 0, 1], false, 1),
            Err(Status::Denied)
        );
        for op in [Operation::Read, Operation::Write, Operation::Seek] {
            assert_eq!(
                Request::decode(
                    [header(op), 1, 0, 0],
                    true,
                    usize::from(op != Operation::Seek)
                ),
                Err(Status::Denied)
            );
        }
    }

    #[test]
    fn rejects_versions_ids_operations_and_capability_counts() {
        for message in [
            [1, 1, 0, 1],
            [(1 << 32) | 99, 1, 0, 1],
            [header(Operation::Open), 0, 0, 1],
        ] {
            assert_eq!(Request::decode(message, true, 1), Err(Status::Malformed));
        }
        for count in [0, 2] {
            assert_eq!(
                Request::decode([header(Operation::Read), 1, 0, 1], false, count),
                Err(Status::Malformed)
            );
        }
        assert_eq!(
            Request::decode([header(Operation::Seek), 1, 0, 0], false, 1),
            Err(Status::Malformed)
        );
    }

    #[test]
    fn checks_full_loan_ranges_before_shared_access() {
        for (offset, bytes) in [(u64::MAX, 1), (4096, 1), (0, 4097), (4097, 0)] {
            assert_eq!(
                Request::decode([header(Operation::Write), 1, offset, bytes], false, 1),
                Err(Status::Range)
            );
        }
        assert!(Request::decode([header(Operation::Read), 1, 4095, 1], false, 1).is_ok());
        for len in [0, 4097, u64::MAX] {
            assert_eq!(
                Request::decode([header(Operation::Open), 1, 0, len], true, 1),
                Err(Status::Range)
            );
        }
    }

    #[test]
    fn rejects_invalid_flags_and_modes_without_silent_downgrades() {
        for flags in [3, EXCLUSIVE, TRUNCATE, 4, 0x100, 0x8000_0000] {
            assert_eq!(
                OpenOptions::decode(u64::from(flags)),
                Err(Status::Malformed)
            );
        }
        assert_eq!(
            OpenOptions::decode((0o666 << 32) | u64::from(READ_WRITE)),
            Err(Status::Malformed)
        );
        assert_eq!(
            OpenOptions::decode((0o10000 << 32) | u64::from(CREATE)),
            Err(Status::Malformed)
        );
        let options =
            OpenOptions::decode((0o666 << 32) | u64::from(CREATE | READ_WRITE | APPEND)).unwrap();
        assert!(options.readable() && options.writable());
        assert!(
            !OpenOptions::decode(u64::from(WRITE_ONLY))
                .unwrap()
                .readable()
        );
        assert!(!OpenOptions::decode(0).unwrap().writable());
    }

    #[test]
    fn signed_seek_rejects_negative_and_overflow_before_state_changes() {
        assert_eq!(seek_position(10, 20, -5, 1), Ok(5));
        assert_eq!(seek_position(10, 20, -5, 2), Ok(15));
        assert_eq!(seek_position(0, 0, i64::MIN, 0), Err(Status::Range));
        assert_eq!(
            seek_position(i64::MAX as u64, 0, 1, 1),
            Err(Status::Overflow)
        );
        assert_eq!(seek_position(u64::MAX, 0, i64::MIN, 1), Ok(i64::MAX as u64));
        assert_eq!(seek_position(0, 0, 0, 3), Err(Status::Malformed));
    }
}
