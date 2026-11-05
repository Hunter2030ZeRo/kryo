use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
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

pub(super) fn check_files() {
    let text = "Cryolite native std file\n".repeat(300);
    expect(fs::write("/probe", &text), 400);
    check(expect(fs::read_to_string("/probe"), 401) == text, 402);

    let content: Vec<u8> = (0..9000).map(|index| (index * 37 + 11) as u8).collect();
    expect(fs::write("/probe", &content), 403);
    check(expect(fs::read("/probe"), 404) == content, 405);
    check(
        fs::read_to_string("/probe").unwrap_err().kind() == ErrorKind::InvalidData,
        406,
    );

    let mut file = expect(
        OpenOptions::new().read(true).write(true).open("/probe"),
        407,
    );
    let mut alias = expect(file.try_clone(), 408);
    let mut independent = expect(File::open("/probe"), 409);
    check(file.as_fd().as_raw_fd() >= 3, 410);
    expect(file.seek(SeekFrom::Start(7)), 411);
    let mut observed = [0; 3];
    expect(file.read_exact(&mut observed), 412);
    check(observed == content[7..10], 413);
    expect(alias.read_exact(&mut observed), 414);
    check(observed == content[10..13], 415);
    expect(independent.read_exact(&mut observed), 416);
    check(observed == content[..3], 417);
    check(expect(file.stream_position(), 418) == 13, 419);
    check(file.seek(SeekFrom::Start(u64::MAX)).is_err(), 420);
    check(expect(alias.stream_position(), 421) == 13, 422);
    check(expect(file.metadata(), 423).len() == 9000, 423);
    check(
        file.set_len(u64::MAX).unwrap_err().kind() == ErrorKind::InvalidInput,
        424,
    );
    check(
        file.sync_all().unwrap_err().kind() == ErrorKind::Unsupported,
        425,
    );

    let owned: OwnedFd = file.into();
    let mut file = File::from(owned);
    expect(file.read_exact(&mut observed), 426);
    check(observed == content[13..16], 427);
    drop(file);
    drop(alias);
    drop(independent);

    let mut append = expect(
        OpenOptions::new().read(true).append(true).open("/probe"),
        428,
    );
    expect(append.seek(SeekFrom::Start(0)), 429);
    let tail = [0x00, 0x7f, 0x80, 0xff];
    expect(append.write_all(&tail), 430);
    expect(append.flush(), 431);
    check(expect(append.stream_position(), 432) == 9004, 433);
    drop(append);
    let observed = expect(fs::read("/probe"), 434);
    check(observed.len() == 9004 && observed[..9000] == content, 435);
    check(observed[9000..] == tail, 436);

    check(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open("/probe")
            .unwrap_err()
            .kind()
            == ErrorKind::AlreadyExists,
        437,
    );
    check(
        expect(fs::read_to_string("/readonly"), 438) == "read-only fixture\n",
        439,
    );
    check(
        OpenOptions::new()
            .write(true)
            .open("/readonly")
            .unwrap_err()
            .kind()
            == ErrorKind::PermissionDenied,
        440,
    );
    check(
        File::open("/missing").unwrap_err().kind() == ErrorKind::NotFound,
        441,
    );
    check(
        OpenOptions::new().open("/probe").unwrap_err().kind() == ErrorKind::InvalidInput,
        442,
    );
    let nul = Path::new(OsStr::from_bytes(b"/probe\0suffix"));
    check(
        File::open(nul).unwrap_err().kind() == ErrorKind::InvalidInput,
        443,
    );
    check(expect(fs::metadata("/probe"), 444).len() == 9004, 444);

    expect(fs::write("/probe", []), 445);
    check(expect(fs::read("/probe"), 446).is_empty(), 447);
    event(230, 1);
}
