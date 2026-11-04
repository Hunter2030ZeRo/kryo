use super::IpcError;
use alloc::vec::Vec;

const NONE: usize = usize::MAX;

/// Runtime retirement identity. Never accepted from a user message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GateKey {
    pub registry: usize,
    pub object: u64,
}

struct Event {
    label: u64,
    key: GateKey,
    pending: bool,
    next: usize,
}

pub(super) struct GateEvents {
    slots: Vec<Option<Event>>,
    tail: usize,
}

impl GateEvents {
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            tail: NONE,
        }
    }

    pub fn reserve(&mut self, label: u64, key: GateKey) -> Result<usize, IpcError> {
        if label == 0
            || self
                .slots
                .iter()
                .flatten()
                .any(|event| event.label == label)
        {
            return Err(IpcError::AlreadyWaiting);
        }
        let slot = if let Some(slot) = self.slots.iter().position(Option::is_none) {
            slot
        } else {
            self.slots.try_reserve(1).map_err(|_| IpcError::Capacity)?;
            self.slots.push(None);
            self.slots.len() - 1
        };
        self.slots[slot] = Some(Event {
            label,
            key,
            pending: false,
            next: NONE,
        });
        Ok(slot)
    }

    pub fn discard(&mut self, slot: usize, key: GateKey) {
        if let Some(event) = self.slots.get(slot).and_then(Option::as_ref) {
            assert_eq!(event.key, key);
            assert!(
                !event.pending,
                "Only unpublished gate rollback may discard a slot"
            );
            self.slots[slot] = None;
        }
    }

    /// Mint reserved this slot and queue link. Final-reference publication cannot allocate.
    pub fn close(&mut self, slot: usize, key: GateKey) {
        let event = self.slots[slot].as_mut().expect("Live gate slot invariant");
        assert_eq!(event.key, key);
        assert!(!event.pending, "Gate closes exactly once");
        event.pending = true;
        // A circular FIFO needs only the tail: tail.next is the head.
        let head = if self.tail == NONE {
            slot
        } else {
            self.slots[self.tail].as_ref().unwrap().next
        };
        self.slots[slot].as_mut().unwrap().next = head;
        if self.tail != NONE {
            self.slots[self.tail].as_mut().unwrap().next = slot;
        }
        self.tail = slot;
    }

    pub const fn has_pending(&self) -> bool {
        self.tail != NONE
    }

    pub fn pop(&mut self) -> Option<(u64, GateKey)> {
        if self.tail == NONE {
            return None;
        }
        let slot = self.slots[self.tail].as_ref().unwrap().next;
        let event = self.slots[slot].take().unwrap();
        if slot == self.tail {
            self.tail = NONE;
        } else {
            self.slots[self.tail].as_mut().unwrap().next = event.next;
        }
        Some((event.label, event.key))
    }

    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            *slot = None;
        }
        self.tail = NONE;
    }
}
