use core::ptr::NonNull;
use core::sync::atomic::{Ordering, fence};

pub const QUEUE_SIZE: u16 = 8;
pub const MAX_BUFFERS: usize = 3;
pub const DESCRIPTOR_OFFSET: u64 = 0;
pub const AVAILABLE_OFFSET: u64 = 256;
pub const USED_OFFSET: u64 = 512;
pub const BOUNCE_OFFSET: u64 = 4096;
pub const BOUNCE_BYTES: u32 = 8192;
pub const REGION_BYTES: u64 = BOUNCE_OFFSET + BOUNCE_BYTES as u64;

const NEXT: u16 = 1;
const WRITE: u16 = 2;
const NO_INTERRUPT: u16 = 1;

/// An offset into a retained memory object, never a DMA or CPU address.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub offset: u64,
    pub length: u32,
    /// Zero means device-readable; one means device-writable.
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buffer {
    pub memory_offset: u64,
    pub bytes: u32,
    pub device_writable: bool,
    pub bounce_offset: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueError {
    InvalidCount,
    InvalidLength,
    InvalidFlags,
    InvalidDirection,
    InvalidRange,
    OverlappingBuffers,
    Capacity,
    InvalidAddress,
    Busy,
    InvalidTicket,
    TicketExhausted,
    InvalidUsedIndex,
    InvalidUsedId,
    InvalidUsedLength,
    Poisoned,
}

/// Immutable metadata copied from a caller before any hardware publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    buffers: [Buffer; MAX_BUFFERS],
    count: usize,
    total: u32,
    writable: u32,
}

impl Plan {
    pub fn new(requests: &[Request], memory_bytes: u64) -> Result<Self, QueueError> {
        if requests.is_empty() || requests.len() > MAX_BUFFERS {
            return Err(QueueError::InvalidCount);
        }
        let mut plan = Self {
            buffers: [Buffer::default(); MAX_BUFFERS],
            count: requests.len(),
            total: 0,
            writable: 0,
        };
        let mut writable = false;
        for (index, request) in requests.iter().enumerate() {
            if request.length == 0 {
                return Err(QueueError::InvalidLength);
            }
            if request.flags > 1 {
                return Err(QueueError::InvalidFlags);
            }
            if writable && request.flags == 0 {
                return Err(QueueError::InvalidDirection);
            }
            writable |= request.flags == 1;
            let end = request
                .offset
                .checked_add(u64::from(request.length))
                .filter(|end| *end <= memory_bytes)
                .ok_or(QueueError::InvalidRange)?;
            if plan.buffers[..index].iter().any(|buffer| {
                request.offset < buffer.memory_offset + u64::from(buffer.bytes)
                    && buffer.memory_offset < end
            }) {
                return Err(QueueError::OverlappingBuffers);
            }
            let total = plan
                .total
                .checked_add(request.length)
                .filter(|total| *total <= BOUNCE_BYTES)
                .ok_or(QueueError::Capacity)?;
            plan.buffers[index] = Buffer {
                memory_offset: request.offset,
                bytes: request.length,
                device_writable: request.flags == 1,
                bounce_offset: plan.total,
            };
            plan.total = total;
            if request.flags == 1 {
                plan.writable += request.length;
            }
        }
        Ok(plan)
    }

    pub fn buffers(&self) -> &[Buffer] {
        &self.buffers[..self.count]
    }

    pub const fn total_bytes(&self) -> u32 {
        self.total
    }

    pub const fn writable_bytes(&self) -> u32 {
        self.writable
    }
}

struct Pending {
    ticket: u64,
    writable: u32,
}

/// One outstanding request through a private split ring and packed bounce RAM.
/// This value never owns or releases backing, translations, or a device.
pub struct Queue {
    base: NonNull<u8>,
    device_base: u64,
    available: u16,
    used: u16,
    next_ticket: u64,
    pending: Option<Pending>,
    poisoned: bool,
}

impl Queue {
    /// # Safety
    /// Retain an exclusive, page-aligned, coherent DMA RAM run of REGION_BYTES
    /// at cpu_address, mapped at device_address, for this value's entire use.
    /// Device access must be stopped during construction. No other CPU writer
    /// or ordinary Rust reference may access the ring while it is live. Only
    /// the device may modify its used ring. Caller code owns payload copies,
    /// device submission and explicit hardware retirement before RAM reuse.
    pub unsafe fn new(cpu_address: u64, device_address: u64) -> Result<Self, QueueError> {
        let cpu = usize::try_from(cpu_address).map_err(|_| QueueError::InvalidAddress)?;
        if cpu_address == 0
            || !cpu_address.is_multiple_of(4096)
            || !device_address.is_multiple_of(4096)
            || cpu.checked_add(REGION_BYTES as usize - 1).is_none()
            || device_address.checked_add(REGION_BYTES - 1).is_none()
        {
            return Err(QueueError::InvalidAddress);
        }
        let mut queue = Self {
            base: NonNull::new(cpu as *mut u8).ok_or(QueueError::InvalidAddress)?,
            device_base: device_address,
            available: 0,
            used: 0,
            next_ticket: 1,
            pending: None,
            poisoned: false,
        };
        // Construction excludes device access and references to this storage.
        unsafe { queue.initialize() };
        Ok(queue)
    }

    pub fn submit(&mut self, plan: &Plan) -> Result<u64, QueueError> {
        if self.poisoned {
            return Err(QueueError::Poisoned);
        }
        if self.pending.is_some() {
            return Err(QueueError::Busy);
        }
        if self.read16(USED_OFFSET + 2) != self.used {
            return Err(self.poison(QueueError::InvalidUsedIndex));
        }
        let next = self
            .next_ticket
            .checked_add(1)
            .ok_or(QueueError::TicketExhausted)?;
        for (index, buffer) in plan.buffers().iter().enumerate() {
            let offset = DESCRIPTOR_OFFSET + index as u64 * 16;
            let has_next = index + 1 < plan.count;
            self.write64(
                offset,
                self.device_base + BOUNCE_OFFSET + u64::from(buffer.bounce_offset),
            );
            self.write32(offset + 8, buffer.bytes);
            self.write16(
                offset + 12,
                if buffer.device_writable { WRITE } else { 0 } | if has_next { NEXT } else { 0 },
            );
            self.write16(offset + 14, if has_next { index as u16 + 1 } else { 0 });
        }
        let ticket = self.next_ticket;
        self.next_ticket = next;
        self.pending = Some(Pending {
            ticket,
            writable: plan.writable,
        });
        self.write16(
            AVAILABLE_OFFSET + 4 + u64::from(self.available % QUEUE_SIZE) * 2,
            0,
        );
        self.available = self.available.wrapping_add(1);
        // Descriptor and payload stores precede publication to the device.
        fence(Ordering::Release);
        self.write16(AVAILABLE_OFFSET + 2, self.available);
        // Complete publication before a caller can ring a transport doorbell.
        fence(Ordering::SeqCst);
        Ok(ticket)
    }

    pub fn poll(&mut self, ticket: u64) -> Result<Option<u32>, QueueError> {
        if self.poisoned {
            return Err(QueueError::Poisoned);
        }
        let pending = self.pending.as_ref().ok_or(QueueError::InvalidTicket)?;
        if ticket != pending.ticket {
            return Err(QueueError::InvalidTicket);
        }
        let current = self.read16(USED_OFFSET + 2);
        match current.wrapping_sub(self.used) {
            0 => return Ok(None),
            1 => {}
            _ => return Err(self.poison(QueueError::InvalidUsedIndex)),
        }
        // Device payload and used-element writes precede its index update.
        fence(Ordering::Acquire);
        let entry = USED_OFFSET + 4 + u64::from(self.used % QUEUE_SIZE) * 8;
        if self.read32(entry) != 0 {
            return Err(self.poison(QueueError::InvalidUsedId));
        }
        let length = self.read32(entry + 4);
        if length > pending.writable {
            return Err(self.poison(QueueError::InvalidUsedLength));
        }
        self.used = current;
        self.pending = None;
        Ok(Some(length))
    }

    /// # Safety
    /// The caller has completed device reset and retired every previous device
    /// access to this run. Resetting software state alone cannot stop DMA.
    pub unsafe fn reset(&mut self) {
        unsafe { self.initialize() };
        self.available = 0;
        self.used = 0;
        self.pending = None;
        self.poisoned = false;
    }

    unsafe fn initialize(&mut self) {
        // The first page is the queue; payload bytes are owned by caller copies.
        unsafe { self.base.as_ptr().write_bytes(0, BOUNCE_OFFSET as usize) };
        self.write16(AVAILABLE_OFFSET, NO_INTERRUPT);
    }

    fn poison(&mut self, error: QueueError) -> QueueError {
        self.poisoned = true;
        error
    }

    fn read16(&self, offset: u64) -> u16 {
        // Every private caller uses an aligned offset inside the queue page.
        u16::from_le(unsafe {
            self.base
                .as_ptr()
                .add(offset as usize)
                .cast::<u16>()
                .read_volatile()
        })
    }

    fn read32(&self, offset: u64) -> u32 {
        u32::from_le(unsafe {
            self.base
                .as_ptr()
                .add(offset as usize)
                .cast::<u32>()
                .read_volatile()
        })
    }

    fn write16(&mut self, offset: u64, value: u16) {
        unsafe {
            self.base
                .as_ptr()
                .add(offset as usize)
                .cast::<u16>()
                .write_volatile(value.to_le())
        };
    }

    fn write32(&mut self, offset: u64, value: u32) {
        unsafe {
            self.base
                .as_ptr()
                .add(offset as usize)
                .cast::<u32>()
                .write_volatile(value.to_le())
        };
    }

    fn write64(&mut self, offset: u64, value: u64) {
        unsafe {
            self.base
                .as_ptr()
                .add(offset as usize)
                .cast::<u64>()
                .write_volatile(value.to_le())
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C, align(4096))]
    struct Storage([u8; REGION_BYTES as usize]);

    #[test]
    fn exhausted_tickets_never_publish_or_wrap_after_reset() {
        let mut storage = Storage([0; REGION_BYTES as usize]);
        // The host owns this RAM; there is no device or concurrent accessor.
        let mut queue = unsafe { Queue::new(storage.0.as_mut_ptr() as u64, 0x1000) }.unwrap();
        let plan = Plan::new(
            &[Request {
                offset: 0,
                length: 1,
                flags: 1,
            }],
            1,
        )
        .unwrap();
        queue.next_ticket = u64::MAX;
        assert_eq!(queue.submit(&plan), Err(QueueError::TicketExhausted));
        assert_eq!(queue.read16(AVAILABLE_OFFSET + 2), 0);
        unsafe { queue.reset() };
        assert_eq!(queue.submit(&plan), Err(QueueError::TicketExhausted));
        assert!(queue.pending.is_none());
    }
}
