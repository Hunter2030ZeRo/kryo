use kryo_interfaces::block::{BLOCK_SIZE, Status};
use kryo_libsys::virtio::{Buffer, DEVICE_WRITE, VERSION_1};
use kryo_libsys::{self as sys, Handle};

const VIEW: u64 = 0x0200_0000;
const TABLE: u64 = 32;
const DATA: u64 = 512;
const STATUS: u64 = 7168;
const READ_ONLY: u64 = 1 << 5;
const HAS_FLUSH: u64 = 1 << 9;
const TIMEOUT_NS: u64 = 2_000_000_000;

pub struct IoFailure {
    pub status: Status,
    pub fatal: bool,
}
impl IoFailure {
    pub const fn ordinary(status: Status) -> Self {
        Self {
            status,
            fatal: false,
        }
    }
}

pub struct Device {
    hardware: Handle,
    memory: Handle,
    features: u64,
    blocks: u64,
}

impl Device {
    pub fn open(hardware: Handle) -> Result<Self, IoFailure> {
        let features =
            sys::virtio_info(hardware, 0).map_err(|_| IoFailure::ordinary(Status::Unavailable))?;
        if features & VERSION_1 == 0 || (features & READ_ONLY == 0 && features & HAS_FLUSH == 0) {
            return Err(IoFailure::ordinary(Status::Unavailable));
        }
        let selected = VERSION_1 | features & (READ_ONLY | HAS_FLUSH);
        sys::virtio_start(hardware, selected)
            .map_err(|_| IoFailure::ordinary(Status::Unavailable))?;
        let sectors =
            sys::virtio_info(hardware, 1).map_err(|_| IoFailure::ordinary(Status::Unavailable))?;
        let memory = sys::memory_create(2).map_err(|_| IoFailure::ordinary(Status::Unavailable))?;
        // This service owns both fixed mappings and retains their memory handle.
        unsafe { sys::memory_map(memory, VIEW, 0, true) }
            .map_err(|_| IoFailure::ordinary(Status::Unavailable))?;
        unsafe { sys::memory_map(memory, VIEW + 4096, 1, true) }
            .map_err(|_| IoFailure::ordinary(Status::Unavailable))?;
        let mut device = Self {
            hardware,
            memory,
            features: selected,
            blocks: sectors / 8,
        };
        if device.blocks < 4 {
            return Err(IoFailure::ordinary(Status::Range));
        }
        device.read(0)?;
        for (index, expected) in b"CRYOLITE-BLOCK-PROBE-V1".iter().enumerate() {
            if get(DATA + index as u64) != *expected {
                return Err(IoFailure::ordinary(Status::Unavailable));
            }
        }
        Ok(device)
    }

    pub const fn blocks(&self) -> u64 {
        self.blocks
    }
    pub const fn read_only(&self) -> bool {
        self.features & READ_ONLY != 0
    }

    pub fn read(&mut self, block: u64) -> Result<(), IoFailure> {
        self.transfer(0, block)
    }
    pub fn write(&mut self, block: u64) -> Result<(), IoFailure> {
        self.transfer(1, block)
    }
    pub fn flush(&mut self) -> Result<(), IoFailure> {
        self.transfer(4, 0)
    }

    pub fn copy_from(&self, source: u64) {
        for offset in 0..BLOCK_SIZE as u64 {
            let byte = unsafe { ((source + offset) as *const u8).read_volatile() };
            put(DATA + offset, byte);
        }
    }
    pub fn copy_to(&self, target: u64) {
        for offset in 0..BLOCK_SIZE as u64 {
            unsafe { ((target + offset) as *mut u8).write_volatile(get(DATA + offset)) };
        }
    }

    fn transfer(&mut self, operation: u32, block: u64) -> Result<(), IoFailure> {
        if operation != 4 && block >= self.blocks {
            return Err(IoFailure::ordinary(Status::Range));
        }
        if operation != 0 && self.read_only() {
            return Err(IoFailure::ordinary(Status::ReadOnly));
        }
        let sector = block
            .checked_mul(8)
            .ok_or(IoFailure::ordinary(Status::Range))?;
        put_u32(0, operation);
        put_u32(4, 0);
        put_u64(8, sector);
        put(STATUS, 0xff);
        self.buffer(
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
        let count = if operation == 4 {
            self.buffer(1, status);
            2
        } else {
            self.buffer(
                1,
                Buffer {
                    offset: DATA,
                    length: BLOCK_SIZE as u32,
                    flags: if operation == 0 { DEVICE_WRITE } else { 0 },
                },
            );
            self.buffer(2, status);
            3
        };
        let ticket = match sys::virtio_submit(self.hardware, self.memory, TABLE, count) {
            Ok(ticket) => ticket,
            Err(_) => return Err(self.uncertain()),
        };
        let deadline = match deadline() {
            Ok(value) => value,
            Err(_) => return Err(self.uncertain()),
        };
        loop {
            match sys::virtio_poll(self.hardware, ticket) {
                Ok(Some(length)) => {
                    let expected = if operation == 0 {
                        BLOCK_SIZE as u32 + 1
                    } else {
                        1
                    };
                    if length != expected {
                        return Err(self.uncertain());
                    }
                    return if get(STATUS) == 0 {
                        Ok(())
                    } else {
                        Err(IoFailure::ordinary(Status::Io))
                    };
                }
                Ok(None) => {
                    if pause(deadline).is_err() {
                        return Err(self.uncertain());
                    }
                }
                Err(_) => return Err(self.uncertain()),
            }
        }
    }

    fn uncertain(&mut self) -> IoFailure {
        let recovered = self
            .reset()
            .and_then(|()| sys::virtio_start(self.hardware, self.features));
        IoFailure {
            status: Status::Uncertain,
            fatal: recovered.is_err(),
        }
    }

    fn reset(&self) -> Result<(), sys::Error> {
        let limit = deadline()?;
        loop {
            if sys::virtio_reset(self.hardware)? {
                return Ok(());
            }
            pause(limit)?;
        }
    }

    pub fn close(self) -> Result<(), sys::Error> {
        self.reset()?;
        unsafe { sys::memory_unmap(VIEW) }?;
        unsafe { sys::memory_unmap(VIEW + 4096) }?;
        sys::close(self.memory)?;
        sys::close(self.hardware)
    }

    fn buffer(&self, index: u64, buffer: Buffer) {
        unsafe { ((VIEW + TABLE + index * 16) as *mut Buffer).write_volatile(buffer) };
    }
}

fn deadline() -> Result<u64, sys::Error> {
    sys::clock()?
        .checked_add(TIMEOUT_NS)
        .ok_or(sys::Error::Overflow)
}
fn pause(deadline: u64) -> Result<(), sys::Error> {
    if sys::clock()? >= deadline {
        return Err(sys::Error::Kernel(-10));
    }
    sys::sleep_for(100_000)
}
fn put(offset: u64, byte: u8) {
    unsafe { ((VIEW + offset) as *mut u8).write_volatile(byte) };
}
fn get(offset: u64) -> u8 {
    unsafe { ((VIEW + offset) as *const u8).read_volatile() }
}
fn put_u32(offset: u64, value: u32) {
    unsafe { ((VIEW + offset) as *mut u32).write_volatile(value.to_le()) };
}
fn put_u64(offset: u64, value: u64) {
    unsafe { ((VIEW + offset) as *mut u64).write_volatile(value.to_le()) };
}
