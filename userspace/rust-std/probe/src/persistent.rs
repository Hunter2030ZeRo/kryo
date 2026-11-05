use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::thread;

use super::{check, event, exit};

const FILE_SIZE: usize = 12 * 1024;

fn expect<T>(result: io::Result<T>, stage: u64) -> T {
    match result {
        Ok(value) => value,
        Err(_) => {
            event(229, stage);
            exit(stage);
        }
    }
}

pub fn run(mode: &[u8]) -> bool {
    match mode {
        b"betafs-write" => write_phase(),
        b"betafs-verify" => verify_phase(),
        b"betafs-lock-owner" => lock_owner(),
        b"betafs-lock-waiter" => lock_waiter(),
        _ => return false,
    }
    true
}

fn contents() -> Vec<u8> {
    let phrase = b"Cryolite: persistent BetAFS data across cold boots.\n";
    (0..FILE_SIZE)
        .map(|index| match index / 4096 {
            0 => phrase[index % phrase.len()],
            1 => ((index / 64) % 16) as u8,
            _ => ((index % 257) * 13) as u8,
        })
        .collect()
}

fn write_phase() {
    let expected = contents();
    let mut file = expect(
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open("/persist"),
        900,
    );
    expect(file.write_all(&expected[..6144]), 901);
    expect(file.seek(SeekFrom::Start(123)), 902);
    expect(file.write_all_at(&expected[6144..], 6144), 903);
    check(expect(file.stream_position(), 904) == 123, 905);
    let snapshot = expect(file.metadata(), 906);
    check(
        snapshot.is_file() && snapshot.len() == FILE_SIZE as u64,
        907,
    );
    let mut observed = [0; 257];
    expect(file.read_exact_at(&mut observed, 4090), 908);
    check(observed == expected[4090..4090 + observed.len()], 909);
    check(expect(file.stream_position(), 910) == 123, 911);

    expect(file.set_len((FILE_SIZE + 4096) as u64), 912);
    let mut zeros = [0xff; 4096];
    expect(file.read_exact_at(&mut zeros, FILE_SIZE as u64), 913);
    check(zeros.iter().all(|byte| *byte == 0), 914);
    check(snapshot.len() == FILE_SIZE as u64, 915);
    expect(file.set_len((FILE_SIZE - 113) as u64), 916);
    expect(file.set_len(FILE_SIZE as u64), 917);
    let mut tail = [0xff; 113];
    expect(file.read_exact_at(&mut tail, (FILE_SIZE - 113) as u64), 918);
    check(tail.iter().all(|byte| *byte == 0), 919);
    expect(
        file.write_all_at(&expected[FILE_SIZE - 113..], (FILE_SIZE - 113) as u64),
        920,
    );
    check(expect(file.stream_position(), 921) == 123, 922);
    let mut sequential = [0; 37];
    expect(file.read_exact(&mut sequential), 923);
    check(sequential == expected[123..160], 924);
    check(expect(file.stream_position(), 925) == 160, 926);
    check(expect(fs::read("/persist"), 927) == expected, 928);

    namespace();
    locks();
    expect(file.sync_data(), 929);
    expect(file.sync_all(), 930);
    expect(expect(File::open("/"), 931).sync_all(), 932);
    event(320, 1);
}

fn verify_phase() {
    check(expect(fs::read("/persist"), 933) == contents(), 934);
    namespace();
    let file = expect(File::open("/persist"), 935);
    expect(file.sync_data(), 936);
    expect(file.sync_all(), 937);
    expect(expect(File::open("/"), 938).sync_all(), 939);
    event(321, 1);
}

fn namespace() {
    let mut directory = expect(fs::read_dir("/"), 940);
    let entries = expect(directory.by_ref().collect::<io::Result<Vec<_>>>(), 941);
    check(entries.len() == 3 && directory.next().is_none(), 942);
    drop(directory);
    for name in [b"persist".as_slice(), b"readonly", b"empty"] {
        check(
            entries
                .iter()
                .filter(|entry| entry.file_name().as_bytes() == name)
                .count()
                == 1,
            943,
        );
    }
    for entry in entries {
        let name = entry.file_name();
        check(entry.path() == Path::new("/").join(&name), 944);
        let metadata = expect(entry.metadata(), 945);
        check(expect(entry.file_type(), 946) == metadata.file_type(), 947);
        match name.as_bytes() {
            b"persist" => check(
                metadata.is_file() && metadata.len() == FILE_SIZE as u64,
                948,
            ),
            b"readonly" => check(
                metadata.is_file() && metadata.len() == 18 && metadata.permissions().readonly(),
                949,
            ),
            b"empty" => check(metadata.is_dir() && !metadata.is_symlink(), 950),
            _ => check(false, 951),
        }
    }
    let mut empty = expect(fs::read_dir("/empty"), 952);
    check(empty.next().is_none() && empty.next().is_none(), 953);
    check(expect(fs::metadata("/empty"), 954).is_dir(), 955);
    check(
        expect(fs::read("/readonly"), 956) == b"read-only fixture\n",
        957,
    );
    check(
        expect(fs::metadata("/readonly"), 958)
            .permissions()
            .readonly(),
        959,
    );
}

fn would_block(result: Result<(), TryLockError>, stage: u64) {
    check(matches!(result, Err(TryLockError::WouldBlock)), stage);
}

fn locks() {
    let owner = expect(File::open("/persist"), 960);
    let contender = expect(File::open("/persist"), 961);
    expect(owner.lock_shared(), 962);
    expect(contender.lock_shared(), 963);
    would_block(owner.try_lock(), 964);
    expect(contender.unlock(), 965);
    would_block(contender.try_lock(), 966);
    expect(owner.unlock(), 967);
    expect(owner.lock(), 968);
    let alias = expect(owner.try_clone(), 969);
    drop(owner);
    would_block(contender.try_lock(), 970);
    drop(alias);
    expect(contender.try_lock().map_err(io::Error::from), 971);
    expect(contender.unlock(), 972);
}

fn lock_owner() -> ! {
    let owner = expect(File::open("/persist"), 980);
    expect(owner.lock(), 981);
    event(324, 1);
    loop {
        thread::park();
    }
}

fn lock_waiter() {
    let waiter = expect(File::open("/persist"), 982);
    event(325, 1);
    expect(waiter.lock(), 983);
    check(
        expect(waiter.metadata(), 984).len() == FILE_SIZE as u64,
        985,
    );
    let mut prefix = [0; 64];
    expect(waiter.read_exact_at(&mut prefix, 0), 986);
    check(prefix == contents()[..prefix.len()], 987);
    expect(waiter.unlock(), 988);
    event(326, 1);
}
