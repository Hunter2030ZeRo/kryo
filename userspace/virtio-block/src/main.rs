#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;
use kryo_libsys::virtio::{Buffer, DEVICE_WRITE, VERSION_1};
use kryo_libsys::{self as sys, Error, Handle};

const VIEW: u64 = 0x0200_0000;
const PAGE_SIZE: u64 = 4096;
const MEMORY_BYTES: u64 = 2 * PAGE_SIZE;
const TABLE: u64 = 32;
const DATA: u64 = 512;
const STATUS: u64 = 7168;
const READ: u32 = 0;
const WRITE: u32 = 1;
const FLUSH: u32 = 4;
const READ_ONLY: u64 = 1 << 5;
const HAS_FLUSH: u64 = 1 << 9;
const TIMEOUT_NS: u64 = 2_000_000_000;
const MARKER: &[u8] = b"CRYOLITE-BLOCK-PROBE-V1";

global_asm!(
    r#"
    .section .text.start,"ax"
    .global _start
    .type _start,@function
_start:
    cld
    xor ebp, ebp
    call {main}
    ud2
    .size _start, .-_start
"#,
    main = sym driver_main,
);

extern "C" fn driver_main(device: u64, mode: u64, read_only: u64) -> ! {
    let device = Handle::from_raw(device).unwrap_or_else(|| sys::exit(200));
    if mode == 2 {
        let denied = Err(Error::Kernel(-3));
        if sys::virtio_info(device, 0).map(|_| ()) != denied
            || sys::virtio_start(device, VERSION_1) != denied
            || sys::virtio_submit(device, device, TABLE, 3).map(|_| ()) != denied
            || sys::virtio_poll(device, 1).map(|_| ()) != denied
            || sys::virtio_reset(device).map(|_| ()) != denied
        {
            sys::exit(240);
        }
        sys::exit(0);
    }
    if mode > 1 {
        sys::exit(201);
    }
    let result = run(device, mode, Handle::from_raw(read_only));
    sys::exit(result.err().unwrap_or(0))
}

fn run(device: Handle, mode: u64, read_only: Option<Handle>) -> Result<(), u64> {
    let features = sys::virtio_info(device, 0).map_err(|_| 202u64)?;
    let queue = sys::virtio_info(device, 2).map_err(|_| 204u64)?;
    if features & VERSION_1 == 0 || queue < 8 {
        return Err(205);
    }
    if sys::virtio_info(device, 3) != Err(Error::Kernel(-6)) {
        return Err(206);
    }
    let selected = VERSION_1 | features & (READ_ONLY | HAS_FLUSH);
    if let Some(read_only) = read_only {
        if sys::virtio_start(read_only, selected) != Err(Error::Kernel(-3)) {
            return Err(207);
        }
        sys::close(read_only).map_err(|_| 208u64)?;
    }
    sys::virtio_start(device, selected).map_err(|_| 209u64)?;
    let capacity = sys::virtio_info(device, 1).map_err(|_| 203u64)?;
    if capacity < 32 {
        return Err(205);
    }
    let transfer = Transfer::new(device)?;
    transfer.reject_invalid_requests()?;

    transfer.clear_data();
    let ticket = transfer.submit(READ, 0)?;
    if sys::virtio_submit(device, transfer.memory, TABLE, 3) != Err(Error::Kernel(-6)) {
        return Err(220);
    }
    // The accepted request owns a private snapshot. Changing the user record
    // cannot redirect DMA or change where completion bytes are copied back.
    transfer.set_buffer(
        1,
        Buffer {
            offset: u64::MAX,
            length: 4096,
            flags: DEVICE_WRITE,
        },
    );
    transfer.finish(ticket, 4097, 0)?;
    if sys::virtio_poll(device, ticket) != Err(Error::Kernel(-6)) {
        return Err(221);
    }
    for (offset, expected) in MARKER.iter().copied().enumerate() {
        if get_byte(DATA + offset as u64) != expected {
            return Err(222);
        }
    }

    if mode == 1 {
        transfer.submit(READ, 0)?;
        // Process retirement must reset the device before it releases either
        // the pending request's memory lease or its private DMA storage.
        return Ok(());
    }

    if features & READ_ONLY == 0 && features & HAS_FLUSH == 0 {
        return Err(223);
    }
    transfer.fill_pattern();
    let ticket = transfer.submit(WRITE, 8)?;
    if features & READ_ONLY != 0 {
        // This deliberate negative request must fail at the read-only device,
        // rather than being reported as a successful write by the transport.
        transfer.finish(ticket, 1, 1)?;
    } else {
        transfer.finish(ticket, 1, 0)?;
        let flush = transfer.submit(FLUSH, 0)?;
        transfer.finish(flush, 1, 0)?;
        transfer.clear_data();
        let read = transfer.submit(READ, 8)?;
        transfer.finish(read, 4097, 0)?;
        for offset in 0..4096 {
            if get_byte(DATA + offset) != pattern(offset) {
                return Err(224);
            }
        }
    }

    reset(device)?;
    let restarted = sys::virtio_info(device, 0).map_err(|_| 236u64)?;
    sys::virtio_start(device, VERSION_1 | restarted & (READ_ONLY | HAS_FLUSH))
        .map_err(|_| 237u64)?;
    let next = transfer.submit(READ, 0)?;
    if next == ticket || sys::virtio_poll(device, ticket) != Err(Error::Kernel(-6)) {
        return Err(238);
    }
    transfer.finish(next, 4097, 0)?;
    reset(device)?;
    transfer.close()?;
    sys::close(device).map_err(|_| 225u64)
}

struct Transfer {
    device: Handle,
    memory: Handle,
}

impl Transfer {
    fn new(device: Handle) -> Result<Self, u64> {
        let memory = sys::memory_create(2).map_err(|_| 210u64)?;
        // This image reserves both addresses and uses raw accesses only. No
        // Rust references survive native calls that copy completion bytes.
        unsafe { sys::memory_map(memory, VIEW, 0, true) }.map_err(|_| 211u64)?;
        unsafe { sys::memory_map(memory, VIEW + PAGE_SIZE, 1, true) }.map_err(|_| 212u64)?;
        Ok(Self { device, memory })
    }

    fn reject_invalid_requests(&self) -> Result<(), u64> {
        if sys::virtio_submit(self.device, self.device, TABLE, 3) != Err(Error::Kernel(-4)) {
            return Err(235);
        }
        self.prepare(READ, 0);
        for count in [0, 4] {
            if !invalid(sys::virtio_submit(self.device, self.memory, TABLE, count)) {
                return Err(213);
            }
        }
        if !invalid(sys::virtio_submit(
            self.device,
            self.memory,
            MEMORY_BYTES,
            3,
        )) {
            return Err(214);
        }
        self.set_buffer(
            1,
            Buffer {
                offset: MEMORY_BYTES - 1,
                length: 4096,
                flags: DEVICE_WRITE,
            },
        );
        if !invalid(sys::virtio_submit(self.device, self.memory, TABLE, 3)) {
            return Err(215);
        }
        self.set_buffer(
            1,
            Buffer {
                offset: DATA,
                length: 4096,
                flags: 2,
            },
        );
        if !invalid(sys::virtio_submit(self.device, self.memory, TABLE, 3)) {
            return Err(216);
        }
        Ok(())
    }

    fn prepare(&self, operation: u32, sector: u64) -> usize {
        put_u32(0, operation);
        put_u32(4, 0);
        put_u64(8, sector);
        put_byte(STATUS, 0xff);
        self.set_buffer(
            0,
            Buffer {
                offset: 0,
                length: 16,
                flags: 0,
            },
        );
        let status = Buffer {
            offset: STATUS,
            length: 1,
            flags: DEVICE_WRITE,
        };
        if operation == FLUSH {
            self.set_buffer(1, status);
            2
        } else {
            self.set_buffer(
                1,
                Buffer {
                    offset: DATA,
                    length: 4096,
                    flags: if operation == READ { DEVICE_WRITE } else { 0 },
                },
            );
            self.set_buffer(2, status);
            3
        }
    }

    fn set_buffer(&self, index: u64, buffer: Buffer) {
        assert!(index < 3);
        // TABLE is aligned for fixed-width Buffer records, inside both owned
        // user mappings. Only this thread writes its request description.
        unsafe { ((VIEW + TABLE + index * 16) as *mut Buffer).write_volatile(buffer) };
    }

    fn submit(&self, operation: u32, sector: u64) -> Result<u64, u64> {
        let count = self.prepare(operation, sector);
        sys::virtio_submit(self.device, self.memory, TABLE, count).map_err(|_| 217u64)
    }

    fn finish(&self, ticket: u64, expected_length: u32, expected_status: u8) -> Result<(), u64> {
        let deadline = deadline()?;
        loop {
            match sys::virtio_poll(self.device, ticket).map_err(|_| 218u64)? {
                Some(length) => {
                    if length != expected_length || get_byte(STATUS) != expected_status {
                        return Err(219);
                    }
                    return Ok(());
                }
                None => pause(deadline)?,
            }
        }
    }

    fn clear_data(&self) {
        for offset in 0..4096 {
            put_byte(DATA + offset, 0);
        }
    }

    fn fill_pattern(&self) {
        for offset in 0..4096 {
            put_byte(DATA + offset, pattern(offset));
        }
    }

    fn close(self) -> Result<(), u64> {
        // reset() completed before mappings or handle references are removed.
        unsafe { sys::memory_unmap(VIEW) }.map_err(|_| 226u64)?;
        unsafe { sys::memory_unmap(VIEW + PAGE_SIZE) }.map_err(|_| 227u64)?;
        sys::close(self.memory).map_err(|_| 228u64)
    }
}

fn reset(device: Handle) -> Result<(), u64> {
    let deadline = deadline()?;
    while !sys::virtio_reset(device).map_err(|_| 229u64)? {
        pause(deadline)?;
    }
    Ok(())
}

fn deadline() -> Result<u64, u64> {
    sys::clock()
        .map_err(|_| 230u64)?
        .checked_add(TIMEOUT_NS)
        .ok_or(231)
}

fn pause(deadline: u64) -> Result<(), u64> {
    if sys::clock().map_err(|_| 232u64)? >= deadline {
        return Err(233);
    }
    sys::sleep_for(100_000).map_err(|_| 234u64)
}

fn pattern(offset: u64) -> u8 {
    (offset as u8).wrapping_mul(37).wrapping_add(11)
}

fn invalid<T>(result: Result<T, Error>) -> bool {
    matches!(result, Err(Error::InvalidArgument | Error::Kernel(-6)))
}

fn put_byte(offset: u64, value: u8) {
    assert!(offset < MEMORY_BYTES);
    unsafe { ((VIEW + offset) as *mut u8).write_volatile(value) };
}

fn get_byte(offset: u64) -> u8 {
    assert!(offset < MEMORY_BYTES);
    unsafe { ((VIEW + offset) as *const u8).read_volatile() }
}

fn put_u32(offset: u64, value: u32) {
    assert!(offset.is_multiple_of(4) && offset + 4 <= MEMORY_BYTES);
    unsafe { ((VIEW + offset) as *mut u32).write_volatile(value.to_le()) };
}

fn put_u64(offset: u64, value: u64) {
    assert!(offset.is_multiple_of(8) && offset + 8 <= MEMORY_BYTES);
    unsafe { ((VIEW + offset) as *mut u64).write_volatile(value.to_le()) };
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    sys::exit(250)
}
