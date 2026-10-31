//! Experimental bootstrap calls; version 0 is not a stable service ABI.
//! Input: RAX=call, RDI=first argument, RSI=second argument.
//! Return: RAX=0 or a negative error, RDX=result. Handles use all 64 bits.
//! IPC carries four u64 words in R8..R11; receive returns its reply token in RDX.

pub const ABI_VERSION: u32 = 0;

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
        for number in 0..=13 {
            assert_eq!(Call::try_from(number).unwrap() as u64, number);
        }
        assert_eq!(Error::InvalidHandle.status(), (-2i64) as u64);
    }
}
