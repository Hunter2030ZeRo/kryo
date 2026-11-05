use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::{check, event, exit};

fn expect<T>(result: io::Result<T>, stage: u64) -> T {
    match result {
        Ok(value) => value,
        Err(_) => {
            event(229, stage);
            exit(stage);
        }
    }
}

pub(super) fn before_files() {
    check(
        fs::metadata("/probe").unwrap_err().kind() == ErrorKind::NotFound,
        700,
    );
    check(!expect(fs::exists("/probe"), 701), 702);
    let link = expect(fs::symlink_metadata("/probe-link"), 703);
    check(
        link.is_symlink() && link.len() == b"/probe".len() as u64,
        704,
    );
    check(link.permissions().readonly(), 705);
    check(!expect(Path::new("/probe-link").try_exists(), 706), 707);
    check(
        fs::metadata("/probe-link").unwrap_err().kind() == ErrorKind::NotFound,
        708,
    );
}

pub(super) fn check_metadata() {
    let handles = usage(1);
    let root = expect(fs::metadata("/"), 710);
    check(root.is_dir() && !root.is_file() && !root.is_symlink(), 711);
    check(!root.permissions().readonly(), 712);
    check(
        Path::new("/").is_dir() && Path::new("/readonly").is_file(),
        713,
    );
    let readonly = expect(fs::metadata("/readonly"), 714);
    check(
        readonly.len() == 18 && readonly.permissions().readonly(),
        715,
    );
    check(expect(fs::exists("/readonly"), 716), 717);
    check(!expect(fs::exists("/missing"), 718), 719);

    let mut file = expect(
        OpenOptions::new().read(true).write(true).open("/probe"),
        720,
    );
    check(file.as_raw_fd() == 3, 721);
    expect(file.write_all(b"metadata"), 722);
    expect(file.seek(SeekFrom::Start(3)), 723);
    let snapshot = expect(fs::metadata("/probe"), 724);
    check(snapshot.len() == expect(file.metadata(), 725).len(), 726);
    check(
        snapshot.is_file() && !snapshot.permissions().readonly(),
        727,
    );
    expect(file.set_len(32), 728);
    check(
        expect(fs::metadata("/probe"), 729).len() == 32 && snapshot.len() == 8,
        730,
    );
    check(expect(file.stream_position(), 731) == 3, 732);
    check(
        snapshot.modified().unwrap_err().kind() == ErrorKind::Unsupported,
        733,
    );
    drop(file);

    let link = expect(fs::symlink_metadata("/probe-link"), 734);
    check(link.is_symlink() && !link.is_file(), 735);
    check(
        Path::new("/probe-link").is_symlink() && Path::new("/probe-link").is_file(),
        736,
    );
    check(expect(fs::metadata("/probe-link"), 737).len() == 32, 738);
    check(
        expect(File::open("/probe-link"), 739)
            .metadata()
            .unwrap()
            .len()
            == 32,
        740,
    );

    let dangling = expect(fs::symlink_metadata("/dangling"), 741);
    check(
        dangling.is_symlink() && dangling.len() == b"/missing".len() as u64,
        742,
    );
    check(!expect(Path::new("/dangling").try_exists(), 743), 744);
    check(
        Path::new("/dangling").is_symlink() && !Path::new("/dangling").exists(),
        745,
    );
    check(
        expect(fs::symlink_metadata("/cycle"), 746).is_symlink(),
        747,
    );
    check(
        fs::metadata("/cycle").unwrap_err().raw_os_error() == Some(32),
        748,
    );
    check(
        fs::exists("/cycle").unwrap_err().raw_os_error() == Some(32),
        749,
    );
    check(
        File::open("/cycle").unwrap_err().raw_os_error() == Some(32),
        750,
    );
    check(
        Path::new("/denied").try_exists().unwrap_err().kind() == ErrorKind::PermissionDenied,
        751,
    );
    check(!Path::new("/denied").exists(), 752);

    let nul = Path::new(OsStr::from_bytes(b"/probe\0suffix"));
    check(
        fs::metadata(nul).unwrap_err().kind() == ErrorKind::InvalidInput,
        753,
    );
    let long = vec![b'x'; 4097];
    check(
        fs::metadata(Path::new(OsStr::from_bytes(&long)))
            .unwrap_err()
            .raw_os_error()
            == Some(37),
        754,
    );
    let byte_path = Path::new(OsStr::from_bytes(b"/\xff"));
    check(
        fs::metadata(byte_path).unwrap_err().kind() == ErrorKind::NotFound,
        755,
    );
    check(!expect(byte_path.try_exists(), 756), 757);
    check(
        fs::metadata("").unwrap_err().kind() == ErrorKind::NotFound,
        758,
    );
    check(usage(0) == 0 && usage(1) == handles && usage(2) == 0, 759);
    event(237, 1);
}

fn usage(resource: u64) -> u64 {
    let mut status = 48u64;
    let used;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") status,
            in("rdi") resource,
            in("rsi") 1u64,
            lateout("rdx") used,
            lateout("r8") _, lateout("r9") _, lateout("r10") _, lateout("r11") _,
            options(nostack),
        );
    }
    check(status == 0, 760);
    used
}
