//! Experimental bootstrap calls; version 0 is not a stable service ABI.
//! Input: RAX=call, RDI=first argument, RSI=second argument.
//! Return: RAX=0 or a negative error, RDX=result. Handles use all 64 bits.
//! IPC carries four u64 words in R8..R11; receive returns its reply token in RDX.
//! EndpointCreate/NotificationCreate take a ChannelFactory handle in RDI and
//! return the new, calling-thread-owned channel handle in RDX.
//! ManifestCreate: RDI=version, RSI=service, RDX=instance, R8=argument.
//! ManifestGrant: RDI=manifest, RSI=role, RDX=source, R8=kind, R9=rights.
//! SpawnManifest: RDI=image, RSI=manifest; child RDI=startup record, RSI=0.
//! StartupRead: RDI=record, RSI=word index. HandleInfo returns kind in RDX,
//! rights in R8. Neither record words nor descriptive kind IDs grant authority.
//! ReceiveFor: RDI=endpoint, RSI=capability capacity, RDX=relative nanoseconds.
//! Its interval begins at kernel admission; absolute deadline calls stay absolute.
//! VmReserve: RDI=bytes, RSI=alignment; RDX=kernel-selected base.
//! VmCommit/VmDecommit: RDI=address, RSI=bytes. Lengths are whole 4 KiB pages.
//! Commit succeeds only with zeroed RW/NX backing; reserved holes have no backing.
//! VmRelease: RDI=exact reservation base, RSI=0. Retirement precedes quota refund.
//! Park/ParkUntil: RDI=private word VA, RSI=u32 expected; timed RDX=deadline.
//! RDX returns 0 after wake or 1 on value mismatch. Wake: RDI=VA, RSI=maximum;
//! RDX returns the number detached. Wake never changes or buffers the word value.
//! ThreadCreateNative: RDI=version, RSI=entry, RDX=argument, R8/R9=stack base/bytes,
//! R10/R11=TLS base/bytes, R12=FS base, R13=guard bytes, R14=flags (zero).
//! Return RDX=thread-control handle. The runtime retains all supplied mappings.

pub const ABI_VERSION: u32 = 0;
pub const NATIVE_THREAD_VERSION: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Call {
    Yield = 0,
    Exit = 1,
    TaskStatus = 2,
    TaskStop = 3,
    Close = 4,
    Duplicate = 5,
    Send = 6,
    Receive = 7,
    Call = 8,
    Reply = 9,
    EndpointClose = 10,
    Wait = 11,
    Signal = 12,
    CancelWait = 13,
    MemoryCreate = 14,
    MemoryMap = 15,
    MemoryUnmap = 16,
    SendCaps = 17,
    CallCaps = 18,
    SetTls = 19,
    ClockRead = 20,
    SleepUntil = 21,
    SendUntil = 22,
    ReceiveUntil = 23,
    CallUntil = 24,
    WaitUntil = 25,
    SendCapsUntil = 26,
    CallCapsUntil = 27,
    Spawn = 28,
    ProcessWait = 29,
    DebugLog = 30,
    ProcessWaitUntil = 31,
    ThreadCreate = 32,
    ThreadExit = 33,
    ThreadJoin = 34,
    ThreadJoinUntil = 35,
    DeviceRead = 36,
    DeviceWrite = 37,
    EndpointCreate = 38,
    NotificationCreate = 39,
    ManifestCreate = 40,
    ManifestGrant = 41,
    SpawnManifest = 42,
    StartupRead = 43,
    HandleInfo = 44,
    ReceiveFor = 45,
    ManifestBudget = 46,
    BudgetInfo = 47,
    BudgetSelf = 48,
    VirtioInfo = 49,
    VirtioStart = 50,
    VirtioSubmit = 51,
    VirtioPoll = 52,
    VirtioReset = 53,
    VmReserve = 54,
    VmCommit = 55,
    VmDecommit = 56,
    VmRelease = 57,
    Park = 58,
    ParkUntil = 59,
    Wake = 60,
    ThreadCreateNative = 61,
}

impl TryFrom<u64> for Call {
    type Error = Error;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Yield),
            1 => Ok(Self::Exit),
            2 => Ok(Self::TaskStatus),
            3 => Ok(Self::TaskStop),
            4 => Ok(Self::Close),
            5 => Ok(Self::Duplicate),
            6 => Ok(Self::Send),
            7 => Ok(Self::Receive),
            8 => Ok(Self::Call),
            9 => Ok(Self::Reply),
            10 => Ok(Self::EndpointClose),
            11 => Ok(Self::Wait),
            12 => Ok(Self::Signal),
            13 => Ok(Self::CancelWait),
            14 => Ok(Self::MemoryCreate),
            15 => Ok(Self::MemoryMap),
            16 => Ok(Self::MemoryUnmap),
            17 => Ok(Self::SendCaps),
            18 => Ok(Self::CallCaps),
            19 => Ok(Self::SetTls),
            20 => Ok(Self::ClockRead),
            21 => Ok(Self::SleepUntil),
            22 => Ok(Self::SendUntil),
            23 => Ok(Self::ReceiveUntil),
            24 => Ok(Self::CallUntil),
            25 => Ok(Self::WaitUntil),
            26 => Ok(Self::SendCapsUntil),
            27 => Ok(Self::CallCapsUntil),
            28 => Ok(Self::Spawn),
            29 => Ok(Self::ProcessWait),
            30 => Ok(Self::DebugLog),
            31 => Ok(Self::ProcessWaitUntil),
            32 => Ok(Self::ThreadCreate),
            33 => Ok(Self::ThreadExit),
            34 => Ok(Self::ThreadJoin),
            35 => Ok(Self::ThreadJoinUntil),
            36 => Ok(Self::DeviceRead),
            37 => Ok(Self::DeviceWrite),
            38 => Ok(Self::EndpointCreate),
            39 => Ok(Self::NotificationCreate),
            40 => Ok(Self::ManifestCreate),
            41 => Ok(Self::ManifestGrant),
            42 => Ok(Self::SpawnManifest),
            43 => Ok(Self::StartupRead),
            44 => Ok(Self::HandleInfo),
            45 => Ok(Self::ReceiveFor),
            46 => Ok(Self::ManifestBudget),
            47 => Ok(Self::BudgetInfo),
            48 => Ok(Self::BudgetSelf),
            49 => Ok(Self::VirtioInfo),
            50 => Ok(Self::VirtioStart),
            51 => Ok(Self::VirtioSubmit),
            52 => Ok(Self::VirtioPoll),
            53 => Ok(Self::VirtioReset),
            54 => Ok(Self::VmReserve),
            55 => Ok(Self::VmCommit),
            56 => Ok(Self::VmDecommit),
            57 => Ok(Self::VmRelease),
            58 => Ok(Self::Park),
            59 => Ok(Self::ParkUntil),
            60 => Ok(Self::Wake),
            61 => Ok(Self::ThreadCreateNative),
            _ => Err(Error::InvalidCall),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Error {
    InvalidCall = 1,
    InvalidHandle = 2,
    PermissionDenied = 3,
    WrongType = 4,
    OutOfResources = 5,
    InvalidArgument = 6,
    PeerClosed = 7,
    Cancelled = 8,
    InvalidReply = 9,
    TimedOut = 10,
    Busy = 11,
}

impl Error {
    pub const fn status(self) -> u64 {
        0u64.wrapping_sub(self as u64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum TaskStatus {
    Runnable = 0,
    Running = 1,
    Blocked = 2,
    Dying = 3,
    Exited = 4,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_calls_are_errors_and_status_does_not_overlap_handle_results() {
        assert_eq!(Call::try_from(u64::MAX), Err(Error::InvalidCall));
        for number in 0..=61 {
            assert_eq!(Call::try_from(number).unwrap() as u64, number);
        }
        assert_eq!(Error::InvalidHandle.status(), (-2i64) as u64);
    }
}
