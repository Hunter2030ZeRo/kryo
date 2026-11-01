use super::{ChannelLease, Completion, Delivery, IpcError, MAX_CAPABILITIES, Message, ReplyToken};
use crate::storage::Slots;
use crate::sync::wait::{WaitId, WaitTable};
use crate::task::TaskId;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
struct Pending {
    sender: TaskId,
    wait: WaitId,
    message: Message,
    call: bool,
    cap_count: usize,
}

#[derive(Clone, Copy)]
struct Receiver {
    wait: WaitId,
    cap_capacity: usize,
}

#[derive(Clone, Copy)]
struct Reply {
    receiver: TaskId,
    caller: TaskId,
    wait: WaitId,
}

struct ReplySlot {
    generation: u32,
    retired: bool,
    reply: Option<Reply>,
}

/// Bounded rendezvous state, serialized with its single WaitTable.
/// This version has one receiver-owner task. SMP needs synchronization around both.
pub struct Endpoint<const QUEUE: usize, const REPLIES: usize> {
    _lease: Option<ChannelLease>,
    owner: TaskId,
    closed: Option<IpcError>,
    receiver: Option<Receiver>,
    senders: Slots<Option<Pending>, QUEUE>,
    queued: usize,
    replies: Slots<ReplySlot, REPLIES>,
}

impl<const QUEUE: usize, const REPLIES: usize> Endpoint<QUEUE, REPLIES> {
    pub const fn new(owner: TaskId) -> Self {
        Self {
            _lease: None,
            owner,
            closed: None,
            receiver: None,
            senders: Slots::new([None; QUEUE]),
            queued: 0,
            replies: Slots::new(
                [const {
                    ReplySlot {
                        generation: 1,
                        retired: false,
                        reply: None,
                    }
                }; REPLIES],
            ),
        }
    }

    pub const fn growing(owner: TaskId) -> Self {
        Self {
            _lease: None,
            owner,
            closed: None,
            receiver: None,
            senders: Slots::growing(),
            queued: 0,
            replies: Slots::limited(u32::MAX as usize),
        }
    }

    pub fn leased(owner: TaskId, lease: ChannelLease) -> Self {
        let mut endpoint = Self::growing(owner);
        endpoint._lease = Some(lease);
        endpoint
    }

    pub const fn is_closed(&self) -> bool {
        self.closed.is_some()
    }

    pub fn send<const WAITS: usize>(
        &mut self,
        sender: TaskId,
        wait: WaitId,
        message: Message,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        self.send_caps(sender, wait, message, 0, waits)
    }

    pub fn send_caps<const WAITS: usize>(
        &mut self,
        sender: TaskId,
        wait: WaitId,
        message: Message,
        cap_count: usize,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        self.submit(
            Pending {
                sender,
                wait,
                message,
                call: false,
                cap_count,
            },
            waits,
        )
    }

    pub fn call<const WAITS: usize>(
        &mut self,
        sender: TaskId,
        wait: WaitId,
        message: Message,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        self.call_caps(sender, wait, message, 0, waits)
    }

    pub fn call_caps<const WAITS: usize>(
        &mut self,
        sender: TaskId,
        wait: WaitId,
        message: Message,
        cap_count: usize,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        self.submit(
            Pending {
                sender,
                wait,
                message,
                call: true,
                cap_count,
            },
            waits,
        )
    }

    pub fn receive<const WAITS: usize>(
        &mut self,
        receiver: TaskId,
        wait: WaitId,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        self.receive_caps(receiver, wait, 0, waits)
    }

    /// The runtime must reserve these handle slots before registering the wait.
    pub fn receive_caps<const WAITS: usize>(
        &mut self,
        receiver: TaskId,
        wait: WaitId,
        cap_capacity: usize,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        if receiver != self.owner {
            return Err(IpcError::PermissionDenied);
        }
        self.check_open()?;
        if cap_capacity > MAX_CAPABILITIES {
            return Err(IpcError::Capacity);
        }
        self.prune(waits);
        self.check_wait(receiver, wait, waits)?;
        if self.receiver.is_some() {
            return Err(IpcError::AlreadyWaiting);
        }
        self.receiver = Some(Receiver { wait, cap_capacity });
        self.pump(waits);
        Ok(())
    }

    pub fn reply<const WAITS: usize>(
        &mut self,
        receiver: TaskId,
        token: ReplyToken,
        message: Message,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        if receiver != self.owner {
            return Err(IpcError::PermissionDenied);
        }
        self.check_open()?;
        self.prune(waits);
        let (index, generation) = token.parts().ok_or(IpcError::InvalidReply)?;
        let slot = self.replies.get(index).ok_or(IpcError::InvalidReply)?;
        if slot.retired || slot.generation != generation {
            return Err(IpcError::InvalidReply);
        }
        let reply = slot.reply.ok_or(IpcError::InvalidReply)?;
        if reply.receiver != receiver {
            return Err(IpcError::PermissionDenied);
        }
        let pending = valid_wait(waits, reply.caller, reply.wait);
        self.retire_reply(index);
        if !pending {
            return Err(IpcError::InvalidReply);
        }
        let completed = waits
            .complete(reply.wait, Completion::Replied(message))
            .is_ok();
        self.pump(waits);
        if completed {
            Ok(())
        } else {
            Err(IpcError::InvalidReply)
        }
    }

    /// Cancel through the object before completing or abandoning its wait elsewhere.
    pub fn cancel<const WAITS: usize>(
        &mut self,
        wait: WaitId,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<Delivery, IpcError> {
        self.cancel_with(wait, IpcError::Cancelled, true, waits)
    }

    /// Detaches the expired operation and preserves whether it was accepted.
    /// Detach every wait due at one sampled timestamp before calling progress;
    /// freeing a reply slot must not deliver another already-expired request.
    pub fn timeout<const WAITS: usize>(
        &mut self,
        wait: WaitId,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<Delivery, IpcError> {
        self.cancel_with(wait, IpcError::TimedOut, false, waits)
    }

    /// Resumes rendezvous after a serialized batch of timeouts is detached.
    pub fn progress<const WAITS: usize>(&mut self, waits: &mut WaitTable<Completion, WAITS>) {
        self.prune(waits);
        self.pump(waits);
    }

    fn cancel_with<const WAITS: usize>(
        &mut self,
        wait: WaitId,
        error: IpcError,
        pump_after: bool,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<Delivery, IpcError> {
        self.prune(waits);
        if self.receiver.is_some_and(|receiver| receiver.wait == wait) {
            self.receiver = None;
            fail(waits, wait, error, Delivery::NotDelivered);
            return Ok(Delivery::NotDelivered);
        }
        if let Some(index) = self.senders[..self.queued]
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.wait == wait))
        {
            self.remove_sender(index);
            fail(waits, wait, error, Delivery::NotDelivered);
            if pump_after {
                self.pump(waits);
            }
            return Ok(Delivery::NotDelivered);
        }
        if let Some(index) = self
            .replies
            .iter()
            .position(|slot| slot.reply.is_some_and(|reply| reply.wait == wait))
        {
            self.retire_reply(index);
            fail(waits, wait, error, Delivery::Accepted);
            if pump_after {
                self.pump(waits);
            }
            return Ok(Delivery::Accepted);
        }
        Err(IpcError::InvalidWait)
    }

    pub fn close<const WAITS: usize>(&mut self, waits: &mut WaitTable<Completion, WAITS>) {
        self.shutdown(IpcError::Closed, waits);
    }

    pub fn task_died<const WAITS: usize>(
        &mut self,
        task: TaskId,
        waits: &mut WaitTable<Completion, WAITS>,
    ) {
        if task == self.owner {
            self.shutdown(IpcError::PeerClosed, waits);
            return;
        }
        let mut index = 0;
        while index < self.queued {
            let pending = self.senders[index].unwrap();
            if pending.sender == task {
                self.remove_sender(index);
                fail(
                    waits,
                    pending.wait,
                    IpcError::PeerClosed,
                    Delivery::NotDelivered,
                );
            } else {
                index += 1;
            }
        }
        for index in 0..self.replies.len() {
            if let Some(reply) = self.replies[index].reply
                && reply.caller == task
            {
                self.retire_reply(index);
                fail(waits, reply.wait, IpcError::PeerClosed, Delivery::Accepted);
            }
        }
        self.prune(waits);
        self.pump(waits);
    }

    fn submit<const WAITS: usize>(
        &mut self,
        request: Pending,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        self.check_open()?;
        if request.cap_count > MAX_CAPABILITIES {
            return Err(IpcError::Capacity);
        }
        self.prune(waits);
        self.check_wait(request.sender, request.wait, waits)?;
        if request.call
            && !self.replies.is_growing()
            && self.replies.iter().all(|slot| slot.retired)
        {
            return Err(IpcError::Capacity);
        }
        self.pump(waits);
        if self.queued == 0
            && self
                .receiver
                .is_some_and(|receiver| request.cap_count > receiver.cap_capacity)
        {
            return Err(IpcError::Capacity);
        }
        if self.queued == 0
            && self.receiver.is_some()
            && (!request.call || self.reply_slot().is_some())
        {
            self.deliver(request, waits)?;
            return Ok(());
        }
        self.senders
            .reserve_len(self.queued + 1, || None)
            .map_err(|_| IpcError::Capacity)?;
        self.senders[self.queued] = Some(request);
        self.queued += 1;
        self.pump(waits);
        Ok(())
    }

    fn pump<const WAITS: usize>(&mut self, waits: &mut WaitTable<Completion, WAITS>) {
        while self.receiver.is_some() && self.queued > 0 {
            let request = self.senders[0].unwrap();
            if request.cap_count > self.receiver.unwrap().cap_capacity {
                self.remove_sender(0);
                fail(
                    waits,
                    request.wait,
                    IpcError::Capacity,
                    Delivery::NotDelivered,
                );
                continue;
            }
            if request.call && self.reply_slot().is_none() {
                if self.replies.is_growing() || self.replies.iter().all(|slot| slot.retired) {
                    self.remove_sender(0);
                    fail(
                        waits,
                        request.wait,
                        IpcError::Capacity,
                        Delivery::NotDelivered,
                    );
                    continue;
                }
                break;
            }
            self.remove_sender(0);
            // Availability was checked without releasing serialized endpoint/wait ownership.
            if self.deliver(request, waits).is_err() {
                fail(
                    waits,
                    request.wait,
                    IpcError::Capacity,
                    Delivery::NotDelivered,
                );
            }
        }
    }

    fn deliver<const WAITS: usize>(
        &mut self,
        request: Pending,
        waits: &mut WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        let receiver = self.receiver.ok_or(IpcError::InvalidWait)?;
        if request.cap_count > receiver.cap_capacity {
            return Err(IpcError::Capacity);
        }
        let reply = if request.call {
            let index = self.reply_slot().ok_or(IpcError::Capacity)?;
            let token =
                ReplyToken::new(index, self.replies[index].generation).ok_or(IpcError::Capacity)?;
            self.replies[index].reply = Some(Reply {
                receiver: self.owner,
                caller: request.sender,
                wait: request.wait,
            });
            Some(token)
        } else {
            None
        };
        self.receiver = None;
        let _ = waits.complete(
            receiver.wait,
            Completion::Received {
                message: request.message,
                reply,
                sender: request.sender,
                sender_wait: request.wait,
                capabilities: request.cap_count,
            },
        );
        if !request.call {
            let _ = waits.complete(request.wait, Completion::Sent);
        }
        Ok(())
    }

    fn prune<const WAITS: usize>(&mut self, waits: &WaitTable<Completion, WAITS>) {
        if self
            .receiver
            .is_some_and(|receiver| !valid_wait(waits, self.owner, receiver.wait))
        {
            self.receiver = None;
        }
        let mut index = 0;
        while index < self.queued {
            let request = self.senders[index].unwrap();
            if valid_wait(waits, request.sender, request.wait) {
                index += 1;
            } else {
                self.remove_sender(index);
            }
        }
        for index in 0..self.replies.len() {
            if self.replies[index]
                .reply
                .is_some_and(|reply| !valid_wait(waits, reply.caller, reply.wait))
            {
                self.retire_reply(index);
            }
        }
    }

    fn check_wait<const WAITS: usize>(
        &self,
        task: TaskId,
        wait: WaitId,
        waits: &WaitTable<Completion, WAITS>,
    ) -> Result<(), IpcError> {
        if !valid_wait(waits, task, wait) {
            return Err(IpcError::InvalidWait);
        }
        if self.receiver.is_some_and(|receiver| receiver.wait == wait)
            || self.senders[..self.queued]
                .iter()
                .any(|entry| entry.is_some_and(|entry| entry.wait == wait))
            || self
                .replies
                .iter()
                .any(|slot| slot.reply.is_some_and(|reply| reply.wait == wait))
        {
            return Err(IpcError::AlreadyWaiting);
        }
        Ok(())
    }

    fn check_open(&self) -> Result<(), IpcError> {
        self.closed.map_or(Ok(()), Err)
    }

    fn reply_slot(&mut self) -> Option<usize> {
        self.replies
            .vacant(
                |slot| !slot.retired && slot.reply.is_none(),
                || ReplySlot {
                    generation: 1,
                    retired: false,
                    reply: None,
                },
            )
            .ok()
    }

    fn retire_reply(&mut self, index: usize) {
        let slot = &mut self.replies[index];
        slot.reply = None;
        if slot.generation == u32::MAX {
            slot.retired = true;
        } else {
            slot.generation += 1;
        }
    }

    fn remove_sender(&mut self, index: usize) -> Pending {
        let result = self.senders[index].take().unwrap();
        for next in index + 1..self.queued {
            self.senders[next - 1] = self.senders[next].take();
        }
        self.queued -= 1;
        self.senders[self.queued] = None;
        result
    }

    fn shutdown<const WAITS: usize>(
        &mut self,
        error: IpcError,
        waits: &mut WaitTable<Completion, WAITS>,
    ) {
        if self.closed.is_some() {
            return;
        }
        self.closed = Some(error);
        if let Some(receiver) = self.receiver.take() {
            fail(waits, receiver.wait, error, Delivery::NotDelivered);
        }
        while self.queued > 0 {
            let request = self.remove_sender(0);
            fail(waits, request.wait, error, Delivery::NotDelivered);
        }
        for index in 0..self.replies.len() {
            if let Some(reply) = self.replies[index].reply {
                self.retire_reply(index);
                fail(waits, reply.wait, error, Delivery::Accepted);
            }
        }
    }
}

fn valid_wait<const WAITS: usize>(
    waits: &WaitTable<Completion, WAITS>,
    task: TaskId,
    wait: WaitId,
) -> bool {
    waits.task(wait) == Ok(task) && waits.is_pending(wait)
}

fn fail<const WAITS: usize>(
    waits: &mut WaitTable<Completion, WAITS>,
    wait: WaitId,
    error: IpcError,
    delivery: Delivery,
) {
    let _ = waits.complete(wait, Completion::Failed { error, delivery });
}
