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
    ReadAt = 5,
    WriteAt = 6,
    Info = 7,
    SetLen = 8,
    Sync = 9,
    PathInfo = 10,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum FileType {
    Regular = 1,
    Directory = 2,
    Symlink = 3,
}

pub const READONLY: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub size: u64,
    pub kind: FileType,
    pub readonly: bool,
}

impl Info {
    pub fn decode(size: u64, attributes: u64) -> Result<Self, Status> {
        let kind = match attributes as u32 {
            1 => FileType::Regular,
            2 => FileType::Directory,
            3 => FileType::Symlink,
            _ => return Err(Status::Malformed),
        };
        let flags = (attributes >> 32) as u32;
        if size > i64::MAX as u64 || flags & !READONLY != 0 {
            return Err(Status::Malformed);
        }
        Ok(Self {
            size,
            kind,
            readonly: flags & READONLY != 0,
        })
    }

    pub const fn attributes(self) -> u64 {
        self.kind as u64
            | if self.readonly {
                (READONLY as u64) << 32
            } else {
                0
            }
    }

    pub const fn response(self, id: u64) -> [u64; 4] {
        [id, Status::Ok as u64, self.size, self.attributes()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum SyncScope {
    All = 0,
    Data = 1,
}

impl TryFrom<u64> for SyncScope {
    type Error = Status;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::All),
            1 => Ok(Self::Data),
            _ => Err(Status::Malformed),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum LookupPolicy {
    Follow = 0,
    NoFollow = 1,
}

impl TryFrom<u64> for LookupPolicy {
    type Error = Status;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Follow),
            1 => Ok(Self::NoFollow),
            _ => Err(Status::Malformed),
        }
    }
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
    SymlinkLoop = 13,
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
            5 => Operation::ReadAt,
            6 => Operation::WriteAt,
            7 => Operation::Info,
            8 => Operation::SetLen,
            9 => Operation::Sync,
            10 => Operation::PathInfo,
            _ => return Err(Status::Malformed),
        };
        if namespace != matches!(operation, Operation::Open | Operation::PathInfo) {
            return Err(Status::Denied);
        }
        let expected = match operation {
            Operation::Open
            | Operation::Read
            | Operation::Write
            | Operation::ReadAt
            | Operation::WriteAt
            | Operation::PathInfo => 1,
            Operation::Seek | Operation::Info | Operation::SetLen | Operation::Sync => 0,
        };
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
            Operation::ReadAt | Operation::WriteAt => {
                if message[3] > PAGE_SIZE {
                    return Err(Status::Range);
                }
                if message[2] > i64::MAX as u64 {
                    return Err(Status::Overflow);
                }
                if operation == Operation::WriteAt
                    && message[2]
                        .checked_add(message[3])
                        .is_none_or(|end| end > i64::MAX as u64)
                {
                    return Err(Status::Overflow);
                }
            }
            Operation::Info => {
                if message[2] != 0 || message[3] != 0 {
                    return Err(Status::Malformed);
                }
            }
            Operation::SetLen => {
                if message[3] != 0 {
                    return Err(Status::Malformed);
                }
                if message[2] > i64::MAX as u64 {
                    return Err(Status::Overflow);
                }
            }
            Operation::Sync => {
                SyncScope::try_from(message[2])?;
                if message[3] != 0 {
                    return Err(Status::Malformed);
                }
            }
            Operation::PathInfo => {
                LookupPolicy::try_from(message[2])?;
                if message[3] == 0 || message[3] > PAGE_SIZE {
                    return Err(Status::Range);
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
        for (op, caps) in [
            (Operation::Read, 1),
            (Operation::Write, 1),
            (Operation::Seek, 0),
            (Operation::ReadAt, 1),
            (Operation::WriteAt, 1),
            (Operation::Info, 0),
            (Operation::SetLen, 0),
            (Operation::Sync, 0),
        ] {
            assert_eq!(
                Request::decode([header(op), 1, 0, 0], true, caps),
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

    #[test]
    fn positioned_io_admits_eof_reads_but_rejects_unrepresentable_writes() {
        assert!(
            Request::decode(
                [header(Operation::ReadAt), 1, i64::MAX as u64, PAGE_SIZE],
                false,
                1
            )
            .is_ok()
        );
        assert!(
            Request::decode(
                [header(Operation::WriteAt), 1, i64::MAX as u64, 0],
                false,
                1
            )
            .is_ok()
        );
        for (operation, offset, bytes, error) in [
            (Operation::ReadAt, 0, PAGE_SIZE + 1, Status::Range),
            (Operation::WriteAt, 0, PAGE_SIZE + 1, Status::Range),
            (Operation::ReadAt, i64::MAX as u64 + 1, 0, Status::Overflow),
            (Operation::WriteAt, i64::MAX as u64, 1, Status::Overflow),
            (Operation::WriteAt, i64::MAX as u64 - 1, 2, Status::Overflow),
        ] {
            assert_eq!(
                Request::decode([header(operation), 1, offset, bytes], false, 1),
                Err(error)
            );
        }
    }

    #[test]
    fn new_operations_require_exact_capability_counts_and_reserved_words() {
        for (operation, argument, length, count) in [
            (Operation::ReadAt, 0, 1, 1),
            (Operation::WriteAt, 0, 1, 1),
            (Operation::Info, 0, 0, 0),
            (Operation::SetLen, 1, 0, 0),
            (Operation::Sync, 0, 0, 0),
            (Operation::Sync, 1, 0, 0),
        ] {
            for caps in 0..=2 {
                let request =
                    Request::decode([header(operation), 1, argument, length], false, caps);
                assert_eq!(request.is_ok(), caps == count);
            }
        }
        for (operation, argument, length, status) in [
            (Operation::Info, 1, 0, Status::Malformed),
            (Operation::Info, 0, 1, Status::Malformed),
            (Operation::SetLen, 0, 1, Status::Malformed),
            (Operation::SetLen, i64::MAX as u64 + 1, 0, Status::Overflow),
            (Operation::Sync, 2, 0, Status::Malformed),
            (Operation::Sync, 0, 1, Status::Malformed),
        ] {
            assert_eq!(
                Request::decode([header(operation), 1, argument, length], false, 0),
                Err(status)
            );
        }
    }

    #[test]
    fn metadata_has_no_unknown_kinds_flags_or_unrepresentable_lengths() {
        for kind in [FileType::Regular, FileType::Directory, FileType::Symlink] {
            for readonly in [false, true] {
                let info = Info {
                    size: i64::MAX as u64,
                    kind,
                    readonly,
                };
                assert_eq!(Info::decode(info.size, info.attributes()), Ok(info));
                assert_eq!(info.response(7), [7, 0, info.size, info.attributes()]);
            }
        }
        for (size, attributes) in [
            (0, 0),
            (0, 4),
            (0, (2 << 32) | 1),
            (0, (1 << 63) | 1),
            (i64::MAX as u64 + 1, 1),
        ] {
            assert_eq!(Info::decode(size, attributes), Err(Status::Malformed));
        }
    }

    #[test]
    fn path_metadata_requires_namespace_authority_and_one_bounded_path_loan() {
        for policy in [LookupPolicy::Follow, LookupPolicy::NoFollow] {
            let message = [header(Operation::PathInfo), 1, policy as u64, 1];
            assert!(Request::decode(message, true, 1).is_ok());
            assert_eq!(Request::decode(message, false, 1), Err(Status::Denied));
            for caps in [0, 2] {
                assert_eq!(Request::decode(message, true, caps), Err(Status::Malformed));
            }
        }
        for length in [0, PAGE_SIZE + 1, u64::MAX] {
            assert_eq!(
                Request::decode([header(Operation::PathInfo), 1, 0, length], true, 1),
                Err(Status::Range)
            );
        }
        assert_eq!(
            Request::decode([header(Operation::PathInfo), 1, 2, 1], true, 1),
            Err(Status::Malformed)
        );
        assert_eq!(
            Request::decode([header(Operation::PathInfo), 0, 0, 1], true, 1),
            Err(Status::Malformed)
        );
    }
}
