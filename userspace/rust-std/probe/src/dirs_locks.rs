use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, ErrorKind, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

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

pub fn check_dirs_locks(concurrent: bool) {
    directories();
    sequential_locks();
    event(238, 1);
    if concurrent {
        blocking_lock();
    }
}

fn directories() {
    let probe_size = expect(fs::metadata("/probe"), 800).len();
    let mut directory = expect(fs::read_dir("/"), 801);
    let entries = expect(directory.by_ref().collect::<io::Result<Vec<_>>>(), 802);
    check(entries.len() == 5, 803);
    check(
        directory.next().is_none() && directory.next().is_none(),
        804,
    );
    drop(directory);

    let names: [&[u8]; 5] = [b"probe", b"readonly", b"probe-link", b"dangling", b"cycle"];
    for name in names {
        check(
            entries
                .iter()
                .filter(|entry| entry.file_name().as_bytes() == name)
                .count()
                == 1,
            805,
        );
    }
    for entry in &entries {
        let name = entry.file_name();
        check(entry.path() == Path::new("/").join(&name), 806);
        let metadata = expect(entry.metadata(), 807);
        let file_type = expect(entry.file_type(), 808);
        check(file_type == metadata.file_type(), 809);
        match name.as_bytes() {
            b"probe" => {
                check(metadata.is_file() && metadata.len() == probe_size, 810);
                check(!metadata.permissions().readonly(), 811);
            }
            b"readonly" => {
                check(metadata.is_file() && metadata.len() == 18, 812);
                check(metadata.permissions().readonly(), 813);
            }
            b"probe-link" | b"dangling" | b"cycle" => {
                check(metadata.is_symlink() && !metadata.is_file(), 814);
                let size = if name.as_bytes() == b"dangling" { 8 } else { 6 };
                check(
                    metadata.len() == size && metadata.permissions().readonly(),
                    815,
                );
            }
            _ => check(false, 816),
        }
    }
    let probe = entries
        .iter()
        .find(|entry| entry.file_name().as_bytes() == b"probe")
        .unwrap();
    let file = expect(OpenOptions::new().write(true).open("/probe"), 817);
    expect(file.set_len(probe_size + 1), 818);
    check(expect(probe.metadata(), 819).len() == probe_size, 820);
    check(
        expect(fs::metadata("/probe"), 821).len() == probe_size + 1,
        822,
    );
    expect(file.set_len(probe_size), 823);
    drop(file);
    drop(entries);

    let mut first = expect(fs::read_dir("/"), 824);
    let mut second = expect(fs::read_dir("/"), 825);
    let first_entry = expect(first.next().unwrap(), 826);
    let second_entry = expect(second.next().unwrap(), 827);
    check(first_entry.file_name() == second_entry.file_name(), 828);
    let first_rest = expect(first.by_ref().collect::<io::Result<Vec<_>>>(), 829);
    let second_rest = expect(second.by_ref().collect::<io::Result<Vec<_>>>(), 830);
    check(first_rest.len() == 4 && second_rest.len() == 4, 831);
    check(
        first_rest
            .iter()
            .zip(&second_rest)
            .all(|(a, b)| a.file_name() == b.file_name()),
        832,
    );
    check(first.next().is_none() && second.next().is_none(), 833);
    drop(first);
    drop(second);

    check(
        fs::read_dir("/probe").unwrap_err().kind() == ErrorKind::NotADirectory,
        834,
    );
    let mut root = expect(File::open("/"), 835);
    check(expect(root.metadata(), 836).is_dir(), 837);
    let mut byte = [0xff];
    check(
        root.read(&mut byte).unwrap_err().kind() == ErrorKind::IsADirectory,
        838,
    );
    check(byte == [0xff] && expect(root.metadata(), 839).is_dir(), 840);
    check(
        root.sync_all().unwrap_err().kind() == ErrorKind::Unsupported,
        841,
    );
    check(
        root.sync_data().unwrap_err().kind() == ErrorKind::Unsupported,
        842,
    );
    check(expect(root.metadata(), 843).is_dir(), 844);
    drop(root);
    let mut probe = expect(File::open("/probe"), 845);
    check(expect(probe.read(&mut byte), 846) == 1, 847);
}

fn would_block(result: Result<(), TryLockError>, stage: u64) {
    check(matches!(result, Err(TryLockError::WouldBlock)), stage);
}

fn sequential_locks() {
    let first = expect(File::open("/probe"), 850);
    let second = expect(File::open("/probe"), 851);
    expect(first.lock_shared(), 852);
    expect(first.lock_shared(), 853);
    expect(first.try_lock_shared().map_err(io::Error::from), 854);
    expect(second.lock_shared(), 855);
    would_block(first.try_lock(), 856);
    check(first.lock().unwrap_err().kind() == ErrorKind::Deadlock, 857);
    expect(second.unlock(), 858);
    would_block(second.try_lock(), 859);
    expect(first.unlock(), 860);
    expect(second.try_lock().map_err(io::Error::from), 861);
    expect(second.lock(), 862);
    expect(second.try_lock().map_err(io::Error::from), 863);
    would_block(first.try_lock_shared(), 864);
    expect(second.unlock(), 865);
    expect(first.try_lock_shared().map_err(io::Error::from), 866);
    expect(first.unlock(), 867);

    expect(first.lock(), 868);
    let alias = expect(first.try_clone(), 869);
    expect(alias.try_lock().map_err(io::Error::from), 870);
    drop(first);
    would_block(second.try_lock(), 871);
    would_block(second.try_lock_shared(), 872);
    drop(alias);
    expect(second.try_lock().map_err(io::Error::from), 873);
    expect(second.unlock(), 874);
    check(expect(second.metadata(), 875).is_file(), 876);
}

fn blocking_lock() {
    let owner = expect(File::open("/probe"), 880);
    let waiter = expect(File::open("/probe"), 881);
    expect(owner.lock(), 882);
    let entered = Arc::new(AtomicBool::new(false));
    let acquired = Arc::new(AtomicBool::new(false));
    let child_entered = entered.clone();
    let child_acquired = acquired.clone();
    let child = expect(
        thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || -> io::Result<()> {
                child_entered.store(true, Ordering::Release);
                waiter.lock()?;
                child_acquired.store(true, Ordering::Release);
                waiter.unlock()
            }),
        883,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (status, used) = wait_usage();
        check(status == 0, 884);
        if entered.load(Ordering::Acquire) && used == 1 {
            break;
        }
        check(Instant::now() < deadline, 885);
        thread::yield_now();
    }
    check(!acquired.load(Ordering::Acquire), 886);
    event(239, 1);
    expect(owner.unlock(), 887);
    match child.join() {
        Ok(result) => expect(result, 888),
        Err(_) => check(false, 889),
    }
    check(acquired.load(Ordering::Acquire), 890);
    let (status, used) = wait_usage();
    check(status == 0 && used == 0, 891);
    event(240, 1);
}

fn wait_usage() -> (i64, u64) {
    let mut status = 48u64;
    let used;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") status,
            in("rdi") 2u64,
            in("rsi") 1u64,
            lateout("rdx") used,
            lateout("r8") _, lateout("r9") _, lateout("r10") _, lateout("r11") _,
            options(nostack),
        );
    }
    (status as i64, used)
}
