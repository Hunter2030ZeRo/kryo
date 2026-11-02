//! Versioned startup records. Identifiers describe policy; handles carry authority.

pub const MAGIC: u64 = u64::from_le_bytes(*b"KRYOSTRT");
pub const VERSION: u64 = 1;
pub const HEADER_WORDS: usize = 7;
pub const ENTRY_WORDS: usize = 4;
pub const MAX_GRANTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub service: u64,
    pub instance: u64,
    pub argument: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub role: u64,
    pub kind: ObjectKind,
    pub rights: u64,
    pub handle: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum ObjectKind {
    TaskControl = 1,
    Memory = 2,
    Endpoint = 3,
    Notification = 4,
    ProcessControl = 5,
    ThreadControl = 6,
    Image = 7,
    DebugConsole = 8,
    Device = 9,
    ChannelFactory = 10,
    LaunchManifest = 11,
    StartupRecord = 12,
    ResourceBudget = 13,
}

impl ObjectKind {
    /// Rights meaningful for this kind, including explicit delegation rights.
    pub const fn allowed_rights(self) -> u64 {
        const INSPECT: u64 = 1;
        const CONTROL: u64 = 2;
        const DUP: u64 = 4;
        const SEND: u64 = 8;
        const RECEIVE: u64 = 16;
        const WAIT: u64 = 32;
        const SIGNAL: u64 = 64;
        const READ: u64 = 128;
        const WRITE: u64 = 256;
        const MAP: u64 = 512;
        const TRANSFER: u64 = 1024;
        const EXECUTE: u64 = 2048;
        const CREATE: u64 = 4096;
        const ASSIGN: u64 = 8192;
        match self {
            Self::TaskControl => INSPECT | CONTROL | DUP | TRANSFER,
            Self::Memory => READ | WRITE | MAP | DUP | TRANSFER,
            Self::Endpoint => SEND | RECEIVE | CONTROL | DUP | TRANSFER,
            Self::Notification => WAIT | SIGNAL | DUP | TRANSFER,
            Self::ProcessControl | Self::ThreadControl => INSPECT | CONTROL | WAIT | DUP | TRANSFER,
            Self::Image => EXECUTE | DUP | TRANSFER,
            Self::DebugConsole => WRITE | DUP | TRANSFER,
            Self::Device => READ | WRITE | CONTROL | DUP | TRANSFER,
            Self::ChannelFactory => CREATE | DUP | TRANSFER,
            Self::LaunchManifest => READ | WRITE,
            Self::StartupRecord => READ,
            Self::ResourceBudget => READ | ASSIGN | DUP | TRANSFER,
        }
    }
}

impl TryFrom<u64> for ObjectKind {
    type Error = ();

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::TaskControl),
            2 => Ok(Self::Memory),
            3 => Ok(Self::Endpoint),
            4 => Ok(Self::Notification),
            5 => Ok(Self::ProcessControl),
            6 => Ok(Self::ThreadControl),
            7 => Ok(Self::Image),
            8 => Ok(Self::DebugConsole),
            9 => Ok(Self::Device),
            10 => Ok(Self::ChannelFactory),
            11 => Ok(Self::LaunchManifest),
            12 => Ok(Self::StartupRecord),
            13 => Ok(Self::ResourceBudget),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_kinds_and_record_limits_are_explicit() {
        for raw in 1..=13 {
            let kind = ObjectKind::try_from(raw).unwrap();
            assert_eq!(kind as u64, raw);
            assert_ne!(kind.allowed_rights(), 0);
            assert_eq!(kind.allowed_rights() & !16383, 0);
        }
        assert!(ObjectKind::try_from(0).is_err());
        assert!(ObjectKind::try_from(14).is_err());
        assert_eq!(HEADER_WORDS + MAX_GRANTS * ENTRY_WORDS, 135);
        assert_eq!(ObjectKind::LaunchManifest.allowed_rights(), 128 | 256);
        assert_eq!(ObjectKind::StartupRecord.allowed_rights(), 128);
        assert_eq!(ObjectKind::ChannelFactory.allowed_rights() & 4096, 4096);
        assert_eq!(ObjectKind::Image.allowed_rights() & 4096, 0);
    }
}
