extern crate cryolite_allocator as _;

mod block;
mod locks;
mod server;

use betafs_core::{Compression, CreateAttributes, FileSystem, ROOT_INODE};
use kryo_libsys::{self as sys, Handle, ObjectKind};
use std::sync::atomic::{AtomicU64, Ordering};

static STAGE: AtomicU64 = AtomicU64::new(1);
static DEBUG: AtomicU64 = AtomicU64::new(0);

unsafe extern "C" {
    fn __cryolite_startup_handle() -> u64;
}

fn run() -> Result<(), u64> {
    let record = Handle::from_raw(unsafe { __cryolite_startup_handle() }).ok_or(270u64)?;
    let startup = sys::startup(record).map_err(|_| 271u64)?;
    if startup.service != 60
        || startup.instance != 1
        || startup.argument & !3 != 0
        || startup.grants().count() != 4
    {
        return Err(272);
    }
    let format = startup.argument & 1 != 0;
    let expected = [
        (1, ObjectKind::DebugConsole, sys::WRITE | sys::DUP),
        (10, ObjectKind::Endpoint, sys::RECEIVE | sys::CONTROL),
        (20, ObjectKind::Endpoint, sys::SEND),
        (21, ObjectKind::Endpoint, sys::SEND),
    ];
    let mut handles = [None; 4];
    for (index, (role, kind, rights)) in expected.into_iter().enumerate() {
        let grant = startup.grant(role).ok_or(273u64)?;
        if grant.kind != kind || grant.rights != rights {
            return Err(274);
        }
        handles[index] = Some(grant.handle);
    }
    let debug = handles[0].ok_or(273u64)?;
    DEBUG.store(debug.raw(), Ordering::Release);
    let endpoint = handles[1].ok_or(273u64)?;
    STAGE.store(2, Ordering::Relaxed);
    let (volume, health) = block::Volume::connect(block::Native::new(
        handles[2].ok_or(273u64)?,
        handles[3].ok_or(273u64)?,
    ))
    .map_err(|_| 275u64)?;
    STAGE.store(3, Ordering::Relaxed);
    // Explicit format is confined to this volume's zero superblocks. Open never
    // falls back to format, and Volume permanently excludes physical block zero.
    let mut fs = if format {
        FileSystem::format(volume)
    } else {
        FileSystem::open(volume)
    }
    .map_err(|_| 276u64)?;
    if format {
        STAGE.store(4, Ordering::Relaxed);
        fs.create_file_with_attributes(
            ROOT_INODE,
            b"readonly",
            b"read-only fixture\n",
            Compression::default(),
            CreateAttributes {
                mode: 0o444,
                uid: 0,
                gid: 0,
            },
        )
        .map_err(|_| 277u64)?;
        fs.mkdir_at(ROOT_INODE, b"empty").map_err(|_| 278u64)?;
        fs.sync().map_err(|_| 279u64)?;
    }
    STAGE.store(5, Ordering::Relaxed);
    sys::debug_log(debug, 310, u64::from(format)).map_err(|_| 280u64)?;
    let mut server = server::Server::new(fs, health, (startup.argument & 2 != 0).then_some(debug));
    server.serve(endpoint)?;
    drop(server);
    sys::close(record).map_err(|_| 281u64)?;
    DEBUG.store(0, Ordering::Release);
    sys::close(debug).map_err(|_| 282u64)?;
    Ok(())
}

fn failed(code: u64) -> ! {
    if let Some(debug) = Handle::from_raw(DEBUG.load(Ordering::Acquire)) {
        let _ = sys::debug_log(debug, 399, code);
    }
    sys::exit(code)
}

fn main() {
    std::panic::set_hook(Box::new(|_| failed(STAGE.load(Ordering::Relaxed))));
    if let Err(code) = run() {
        failed(code);
    }
}
