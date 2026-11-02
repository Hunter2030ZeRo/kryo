use core::cell::UnsafeCell;
use core::sync::atomic::{Ordering, fence};
use kryo_core::device::queue::{
    AVAILABLE_OFFSET, BOUNCE_BYTES, BOUNCE_OFFSET, MAX_BUFFERS, Plan, QUEUE_SIZE, Queue,
    QueueError, REGION_BYTES, Request, USED_OFFSET,
};

#[repr(C, align(4096))]
struct Storage(UnsafeCell<[u8; REGION_BYTES as usize]>);

impl Storage {
    fn new() -> Box<Self> {
        Box::new(Self(UnsafeCell::new([0xa5; REGION_BYTES as usize])))
    }

    fn address(&self) -> u64 {
        self.0.get().cast::<u8>() as u64
    }

    fn read16(&self, offset: u64) -> u16 {
        u16::from_le(unsafe { ((self.address() + offset) as *const u16).read_volatile() })
    }

    fn read32(&self, offset: u64) -> u32 {
        u32::from_le(unsafe { ((self.address() + offset) as *const u32).read_volatile() })
    }

    fn read64(&self, offset: u64) -> u64 {
        u64::from_le(unsafe { ((self.address() + offset) as *const u64).read_volatile() })
    }

    fn write16(&self, offset: u64, value: u16) {
        unsafe { ((self.address() + offset) as *mut u16).write_volatile(value.to_le()) };
    }

    fn write32(&self, offset: u64, value: u32) {
        unsafe { ((self.address() + offset) as *mut u32).write_volatile(value.to_le()) };
    }

    fn complete(&self, id: u32, length: u32) {
        let used = self.read16(USED_OFFSET + 2);
        let entry = USED_OFFSET + 4 + u64::from(used % QUEUE_SIZE) * 8;
        self.write32(entry, id);
        self.write32(entry + 4, length);
        fence(Ordering::Release);
        self.write16(USED_OFFSET + 2, used.wrapping_add(1));
    }

    fn queue(&self) -> Queue {
        // This host model owns coherent, aligned RAM and modifies its used ring
        // only through the simulated device. No actual DMA or hardware runs.
        unsafe { Queue::new(self.address(), 0x1_0000_0000) }.unwrap()
    }
}

fn request(offset: u64, length: u32, writable: bool) -> Request {
    Request {
        offset,
        length,
        flags: u32::from(writable),
    }
}

fn plan() -> Plan {
    Plan::new(
        &[
            request(0, 16, false),
            request(512, 4096, true),
            request(8191, 1, true),
        ],
        8192,
    )
    .unwrap()
}

#[test]
fn plan_snapshots_bounded_offsets_and_packs_device_storage() {
    let mut requests = [request(128, 16, false), request(4096, 512, true)];
    let plan = Plan::new(&requests, 8192).unwrap();
    requests[0].offset = u64::MAX;
    assert_eq!(requests[0].offset, u64::MAX);
    assert_eq!(plan.buffers().len(), 2);
    assert_eq!(plan.total_bytes(), 528);
    assert_eq!(plan.writable_bytes(), 512);
    assert_eq!(plan.buffers()[0].memory_offset, 128);
    assert_eq!(plan.buffers()[0].bounce_offset, 0);
    assert_eq!(plan.buffers()[1].memory_offset, 4096);
    assert_eq!(plan.buffers()[1].bounce_offset, 16);
}

#[test]
fn empty_or_oversized_descriptor_lists_are_rejected() {
    assert_eq!(Plan::new(&[], 8192), Err(QueueError::InvalidCount));
    assert_eq!(
        Plan::new(&[request(0, 1, false); MAX_BUFFERS + 1], 8192),
        Err(QueueError::InvalidCount)
    );
}

#[test]
fn zero_lengths_unknown_flags_and_reversed_direction_are_rejected() {
    assert_eq!(
        Plan::new(&[request(0, 0, false)], 8192),
        Err(QueueError::InvalidLength)
    );
    for flags in [2, 3, u32::MAX] {
        assert_eq!(
            Plan::new(
                &[Request {
                    flags,
                    ..request(0, 1, false)
                }],
                8192
            ),
            Err(QueueError::InvalidFlags)
        );
    }
    assert_eq!(
        Plan::new(&[request(0, 16, true), request(16, 16, false)], 8192),
        Err(QueueError::InvalidDirection)
    );
}

#[test]
fn whole_memory_range_and_checked_arithmetic_are_enforced() {
    for (offset, length, memory_bytes) in [(0, 1, 0), (8191, 2, 8192), (u64::MAX, 1, u64::MAX)] {
        assert_eq!(
            Plan::new(&[request(offset, length, true)], memory_bytes),
            Err(QueueError::InvalidRange)
        );
    }
    assert!(Plan::new(&[request(8191, 1, true)], 8192).is_ok());
}

#[test]
fn overlap_is_rejected_but_adjacent_ranges_and_exact_capacity_are_valid() {
    assert_eq!(
        Plan::new(&[request(0, 16, false), request(15, 2, true)], 8192),
        Err(QueueError::OverlappingBuffers)
    );
    let adjacent = Plan::new(
        &[request(0, 16, false), request(16, BOUNCE_BYTES - 16, true)],
        u64::from(BOUNCE_BYTES),
    )
    .unwrap();
    assert_eq!(adjacent.total_bytes(), BOUNCE_BYTES);
    assert_eq!(
        Plan::new(
            &[request(0, 16, false), request(16, BOUNCE_BYTES, true)],
            u64::from(BOUNCE_BYTES) + 16,
        ),
        Err(QueueError::Capacity)
    );
}

#[test]
fn initialization_sets_polling_flags_and_preserves_bounce_payload() {
    let storage = Storage::new();
    let _queue = storage.queue();
    assert_eq!(storage.read16(AVAILABLE_OFFSET), 1);
    assert_eq!(storage.read16(AVAILABLE_OFFSET + 2), 0);
    assert_eq!(storage.read32(USED_OFFSET), 0);
    assert_eq!(storage.read64(0), 0);
    assert_eq!(storage.read64(BOUNCE_OFFSET), 0xa5a5_a5a5_a5a5_a5a5);
}

#[test]
fn hardware_descriptors_only_use_owned_packed_device_addresses() {
    let storage = Storage::new();
    let mut queue = storage.queue();
    let plan = plan();
    let ticket = queue.submit(&plan).unwrap();
    assert_eq!(ticket, 1);
    let bounce = 0x1_0000_0000 + BOUNCE_OFFSET;
    for (index, (address, length, flags, next)) in [
        (bounce, 16, 1, 1),
        (bounce + 16, 4096, 3, 2),
        (bounce + 4112, 1, 2, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let offset = index as u64 * 16;
        assert_eq!(storage.read64(offset), address);
        assert_eq!(storage.read32(offset + 8), length);
        assert_eq!(storage.read16(offset + 12), flags);
        assert_eq!(storage.read16(offset + 14), next);
    }
    assert_eq!(storage.read16(AVAILABLE_OFFSET + 2), 1);
    assert_eq!(storage.read16(AVAILABLE_OFFSET + 4), 0);
    assert_eq!(queue.poll(ticket), Ok(None));
}

#[test]
fn even_large_user_offsets_never_become_device_addresses() {
    let storage = Storage::new();
    let mut queue = storage.queue();
    let plan = Plan::new(&[request(u64::MAX - 16, 16, false)], u64::MAX).unwrap();
    let ticket = queue.submit(&plan).unwrap();
    assert_eq!(storage.read64(0), 0x1_0000_0000 + BOUNCE_OFFSET);
    storage.complete(0, 0);
    assert_eq!(queue.poll(ticket), Ok(Some(0)));
}

#[test]
fn only_matching_live_ticket_can_consume_completion() {
    let storage = Storage::new();
    let mut queue = storage.queue();
    let plan = plan();
    let ticket = queue.submit(&plan).unwrap();
    assert_eq!(queue.submit(&plan), Err(QueueError::Busy));
    assert_eq!(queue.poll(ticket + 1), Err(QueueError::InvalidTicket));
    assert_eq!(queue.poll(ticket), Ok(None));
    storage.complete(0, plan.writable_bytes());
    assert_eq!(queue.poll(ticket), Ok(Some(plan.writable_bytes())));
    assert_eq!(queue.poll(ticket), Err(QueueError::InvalidTicket));
    let next = queue.submit(&plan).unwrap();
    assert_ne!(ticket, next);
    assert_eq!(queue.poll(ticket), Err(QueueError::InvalidTicket));
}

#[test]
fn malformed_device_head_id_poisons_queue_until_hardware_reset() {
    for id in [1, u32::MAX] {
        let storage = Storage::new();
        let mut queue = storage.queue();
        let plan = plan();
        let ticket = queue.submit(&plan).unwrap();
        storage.complete(id, 1);
        assert_eq!(queue.poll(ticket), Err(QueueError::InvalidUsedId));
        assert_eq!(queue.poll(ticket), Err(QueueError::Poisoned));
        assert_eq!(queue.submit(&plan), Err(QueueError::Poisoned));
        // The simulated device has stopped; real callers must finish reset.
        unsafe { queue.reset() };
        assert!(queue.submit(&plan).is_ok());
    }
}

#[test]
fn completion_cannot_exceed_writable_bytes_even_if_total_is_larger() {
    let storage = Storage::new();
    let mut queue = storage.queue();
    let plan = Plan::new(&[request(0, 16, false), request(16, 1, true)], 17).unwrap();
    let ticket = queue.submit(&plan).unwrap();
    storage.complete(0, 2);
    assert_eq!(queue.poll(ticket), Err(QueueError::InvalidUsedLength));
    assert_eq!(queue.submit(&plan), Err(QueueError::Poisoned));
}

#[test]
fn zero_and_partial_device_writes_are_bounded_valid_completions() {
    let storage = Storage::new();
    let mut queue = storage.queue();
    let plan = plan();
    for length in [0, 1, plan.writable_bytes() - 1, plan.writable_bytes()] {
        let ticket = queue.submit(&plan).unwrap();
        storage.complete(0, length);
        assert_eq!(queue.poll(ticket), Ok(Some(length)));
    }
}

#[test]
fn unexpected_completion_count_poisons_instead_of_indexing_untrusted_slot() {
    for current in [2, u16::MAX] {
        let storage = Storage::new();
        let mut queue = storage.queue();
        let ticket = queue.submit(&plan()).unwrap();
        storage.write16(USED_OFFSET + 2, current);
        assert_eq!(queue.poll(ticket), Err(QueueError::InvalidUsedIndex));
        assert_eq!(queue.submit(&plan()), Err(QueueError::Poisoned));
    }
}

#[test]
fn unsolicited_completion_before_submission_is_rejected() {
    let storage = Storage::new();
    let mut queue = storage.queue();
    storage.write16(USED_OFFSET + 2, 1);
    assert_eq!(queue.submit(&plan()), Err(QueueError::InvalidUsedIndex));
    assert_eq!(queue.submit(&plan()), Err(QueueError::Poisoned));
}

#[test]
fn software_reset_discards_stopped_work_without_reusing_old_tickets() {
    let storage = Storage::new();
    let mut queue = storage.queue();
    let old = queue.submit(&plan()).unwrap();
    // Only the device model runs here, and it has no outstanding DMA accesses.
    unsafe { queue.reset() };
    assert_eq!(storage.read16(AVAILABLE_OFFSET + 2), 0);
    assert_eq!(storage.read16(USED_OFFSET + 2), 0);
    let next = queue.submit(&plan()).unwrap();
    assert_ne!(old, next);
    assert_eq!(queue.poll(old), Err(QueueError::InvalidTicket));
    storage.complete(0, 1);
    assert_eq!(queue.poll(next), Ok(Some(1)));
}

#[test]
fn ring_indices_wrap_without_confusing_pending_generation() {
    let storage = Storage::new();
    let mut queue = storage.queue();
    let plan = Plan::new(&[request(0, 1, true)], 1).unwrap();
    for index in 0..=u16::MAX as u64 + 1 {
        let ticket = queue.submit(&plan).unwrap();
        assert_eq!(ticket, index + 1);
        storage.complete(0, 1);
        assert_eq!(queue.poll(ticket), Ok(Some(1)));
    }
    assert_eq!(storage.read16(AVAILABLE_OFFSET + 2), 1);
    assert_eq!(storage.read16(USED_OFFSET + 2), 1);
}

#[test]
fn invalid_cpu_and_device_spans_are_rejected_before_access() {
    let storage = Storage::new();
    for cpu in [0, storage.address() + 1, u64::MAX - 4095] {
        assert!(matches!(
            unsafe { Queue::new(cpu, 0x1000) },
            Err(QueueError::InvalidAddress)
        ));
    }
    for device in [1, u64::MAX - 4095] {
        assert!(matches!(
            unsafe { Queue::new(storage.address(), device) },
            Err(QueueError::InvalidAddress)
        ));
    }
}
