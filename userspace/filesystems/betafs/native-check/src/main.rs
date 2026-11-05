extern crate cryolite_allocator as _;

use betafs_block::{Block, BlockDevice};
use betafs_codec::{Codec, decode, encode};
use betafs_core::FileSystem;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static STAGE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct RamDevice(Arc<Mutex<Vec<Block>>>);

impl BlockDevice for RamDevice {
    fn block_count(&self) -> u64 {
        self.0.lock().unwrap().len() as u64
    }

    fn read_block(&self, id: u64, output: &mut Block) -> io::Result<()> {
        let data = self.0.lock().unwrap();
        let block = data.get(id as usize).ok_or(io::ErrorKind::UnexpectedEof)?;
        output.copy_from_slice(block);
        Ok(())
    }

    fn write_block(&mut self, id: u64, input: &Block) -> io::Result<()> {
        let mut data = self.0.lock().unwrap();
        let block = data
            .get_mut(id as usize)
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        block.copy_from_slice(input);
        Ok(())
    }

    fn sync(&mut self) -> io::Result<()> {
        // This model checks engine execution; durable media is tested separately.
        Ok(())
    }
}

unsafe extern "C" {
    fn __cryolite_debug_handle() -> u64;
}

fn event(number: u64, value: u64) {
    let status: u64;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") 30u64 => status,
            in("rdi") __cryolite_debug_handle(),
            in("rsi") number,
            inlateout("r8") value => _,
            lateout("rdx") _, lateout("r9") _, lateout("r10") _, lateout("r11") _,
            options(nostack),
        );
    }
    if status != 0 {
        std::process::exit(255);
    }
}

fn main() {
    std::panic::set_hook(Box::new(|_| {
        event(399, STAGE.load(Ordering::Relaxed));
        std::process::exit(1);
    }));
    let input = vec![42; 4096];
    for codec in [Codec::Raw, Codec::Zstd, Codec::Lz4, Codec::OpenZl] {
        STAGE.store(10 + codec as u64, Ordering::Relaxed);
        let (actual, encoded) = encode(&input, codec).unwrap();
        assert_eq!(actual, codec);
        assert_eq!(decode(&encoded, actual, input.len()).unwrap(), input);
    }
    STAGE.store(100, Ordering::Relaxed);
    let device = RamDevice(Arc::new(Mutex::new(vec![[0; 4096]; 256])));
    let mut fs = FileSystem::format(device.clone()).unwrap();
    for (path, codec) in [
        ("/raw", Codec::Raw),
        ("/zstd", Codec::Zstd),
        ("/lz4", Codec::Lz4),
        ("/openzl", Codec::OpenZl),
    ] {
        STAGE.store(110 + codec as u64, Ordering::Relaxed);
        fs.create(path, &input, codec).unwrap();
        assert_eq!(fs.read(path).unwrap(), input);
    }
    fs.sync().unwrap();
    drop(fs);
    STAGE.store(120, Ordering::Relaxed);
    let fs = FileSystem::open(device).unwrap();
    for path in ["/raw", "/zstd", "/lz4", "/openzl"] {
        assert_eq!(fs.read(path).unwrap(), input);
    }
    event(300, 15);
}
