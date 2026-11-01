//! Use one persistent WaitTable per runtime and bind each pending wait to at most
//! one object. Detach waits before abandoning them. Reply tokens are endpoint-local.
//! Schedule wakeups only after serialized object and wait mutations return.
//! Capability owners stay in runtime batches. Commit a Received batch into reserved
//! handles before waking the receiver or releasing the sender batch.

use crate::process::ExitStatus;
use crate::sync::wait::WaitId;
use crate::task::TaskId;

mod budget;
mod endpoint;
mod notification;

pub use budget::{ChannelBudget, ChannelLease};
pub use endpoint::Endpoint;
pub use notification::Notification;

pub const MAX_CAPABILITIES: usize = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Message(pub [u64; 4]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    NotDelivered,
    Accepted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpcError {
    Closed,
    Capacity,
    InvalidWait,
    AlreadyWaiting,
    PermissionDenied,
    InvalidReply,
    Cancelled,
    TimedOut,
    PeerClosed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplyToken(u64);

impl ReplyToken {
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }
    pub const fn as_raw(self) -> u64 {
        self.0
    }

    pub(super) fn new(index: usize, generation: u32) -> Option<Self> {
        let index = u32::try_from(index).ok()?.checked_add(1)?;
        Some(Self((u64::from(generation) << 32) | u64::from(index)))
    }

    pub(super) fn parts(self) -> Option<(usize, u32)> {
        let index = (self.0 as u32).checked_sub(1)? as usize;
        let generation = (self.0 >> 32) as u32;
        (generation != 0).then_some((index, generation))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    Sent,
    Received {
        message: Message,
        reply: Option<ReplyToken>,
        sender: TaskId,
        sender_wait: WaitId,
        capabilities: usize,
    },
    Replied(Message),
    Notified,
    Slept,
    ProcessExited(ExitStatus),
    Failed {
        error: IpcError,
        delivery: Delivery,
    },
}
