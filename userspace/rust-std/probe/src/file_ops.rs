use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
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

pub(super) fn check_file_ops(concurrent: bool) {
    metadata_and_resize();
    positioned_and_append();
    positioned_aliases(concurrent);
    if !concurrent {
        quota_recovery();
    }
    event(233, 1);
}

fn metadata_and_resize() {
    let mut file = expect(
        OpenOptions::new().read(true).write(true).open("/probe"),
        500,
    );
    let contents: Vec<_> = (0..64u8).collect();
    expect(file.write_all(&contents), 501);
    expect(file.seek(SeekFrom::Start(17)), 502);
    let snapshot = expect(file.metadata(), 503);
    check(snapshot.len() == 64 && snapshot.is_file(), 504);
    check(!snapshot.is_dir() && !snapshot.is_symlink(), 505);
    check(!snapshot.permissions().readonly(), 506);
    check(
        snapshot.modified().unwrap_err().kind() == ErrorKind::Unsupported,
        507,
    );
    check(
        snapshot.accessed().unwrap_err().kind() == ErrorKind::Unsupported,
        508,
    );
    check(
        snapshot.created().unwrap_err().kind() == ErrorKind::Unsupported,
        509,
    );
    check(expect(file.stream_len(), 510) == 64, 511);
    check(expect(file.stream_position(), 512) == 17, 513);

    let readonly_open = expect(File::open("/probe"), 514);
    check(
        !expect(readonly_open.metadata(), 515)
            .permissions()
            .readonly(),
        516,
    );
    let readonly_file = expect(File::open("/readonly"), 517);
    let info = expect(readonly_file.metadata(), 518);
    check(info.permissions().readonly() && info.is_file(), 519);
    check(info.len() == b"read-only fixture\n".len() as u64, 520);
    drop(readonly_file);
    drop(readonly_open);
    let writeonly = expect(OpenOptions::new().write(true).open("/probe"), 521);
    check(expect(writeonly.metadata(), 522).len() == 64, 523);
    drop(writeonly);

    expect(file.set_len(12), 524);
    expect(file.set_len(32), 525);
    let mut observed = [0xff; 32];
    expect(file.read_exact_at(&mut observed, 0), 526);
    check(observed[..12] == contents[..12], 527);
    check(observed[12..].iter().all(|byte| *byte == 0), 528);
    check(
        snapshot.len() == 64 && expect(file.stream_len(), 529) == 32,
        530,
    );
    check(expect(file.stream_position(), 531) == 17, 532);
    check(
        file.set_len(16_385).unwrap_err().kind() == ErrorKind::StorageFull,
        533,
    );
    check(expect(file.stream_len(), 534) == 32, 535);
    check(
        file.sync_all().unwrap_err().kind() == ErrorKind::Unsupported,
        536,
    );
    check(
        file.sync_data().unwrap_err().kind() == ErrorKind::Unsupported,
        537,
    );
    check(expect(file.stream_position(), 538) == 17, 539);
    drop(file);
    event(232, 1);
}

fn positioned_and_append() {
    let mut file = expect(
        OpenOptions::new().read(true).write(true).open("/probe"),
        540,
    );
    expect(file.set_len(0), 541);
    expect(file.seek(SeekFrom::Start(3)), 542);
    expect(file.write_all_at(b"abc", 9), 543);
    let mut observed = [0xff; 12];
    expect(file.read_exact_at(&mut observed, 0), 544);
    check(observed[..9] == [0; 9] && &observed[9..] == b"abc", 545);
    check(expect(file.stream_position(), 546) == 3, 547);
    let mut byte = [0xff];
    check(
        expect(file.read_at(&mut byte, i64::MAX as u64), 548) == 0,
        549,
    );
    check(byte == [0xff], 550);
    check(file.write_at(b"x", u64::MAX).is_err(), 551);
    check(expect(file.stream_position(), 552) == 3, 553);
    drop(file);

    let mut append = expect(
        OpenOptions::new().read(true).append(true).open("/probe"),
        554,
    );
    expect(append.seek(SeekFrom::Start(2)), 555);
    expect(append.write_all_at(b"XY", 0), 556);
    check(expect(append.stream_len(), 557) == 12, 558);
    check(expect(append.stream_position(), 559) == 2, 560);
    let mut prefix = [0; 2];
    expect(append.read_exact_at(&mut prefix, 0), 561);
    check(&prefix == b"XY", 562);
    expect(append.write_all(b"!"), 563);
    check(expect(append.stream_position(), 564) == 13, 565);
    expect(append.set_len(12), 566);
    check(expect(append.stream_position(), 567) == 13, 568);
    check(expect(append.stream_len(), 569) == 12, 570);
}

fn positioned_aliases(concurrent: bool) {
    let mut file = expect(
        OpenOptions::new().read(true).write(true).open("/probe"),
        580,
    );
    expect(file.set_len(8192), 581);
    expect(file.seek(SeekFrom::Start(37)), 582);
    let mut workers = Vec::new();
    for lane in 0..2u64 {
        let alias = expect(file.try_clone(), 583);
        if !concurrent {
            expect(check_lane(&alias, lane), 584);
            continue;
        }
        workers.push(expect(
            thread::Builder::new()
                .stack_size(256 * 1024)
                .spawn(move || check_lane(&alias, lane)),
            584,
        ));
    }
    for worker in workers {
        match worker.join() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                event(229, 585_000 + error.raw_os_error().unwrap_or(999) as u64);
                exit(585);
            }
            Err(_) => {
                event(229, 585);
                exit(585);
            }
        }
    }
    check(expect(file.stream_position(), 586) == 37, 587);
    check(expect(file.stream_len(), 588) == 8192, 589);
    let mut observed = [0; 8];
    expect(file.read_exact(&mut observed), 590);
    check(expect(file.stream_position(), 591) == 45, 592);
    if concurrent {
        event(234, 1);
    }
}

fn check_lane(file: &File, lane: u64) -> io::Result<()> {
    for index in 0..16u64 {
        let offset = lane * 4096 + index * 8;
        let expected = ((lane + 1) << 32 | index).to_le_bytes();
        file.write_all_at(&expected, offset)?;
        thread::yield_now();
        let mut observed = [0; 8];
        file.read_exact_at(&mut observed, offset)?;
        if observed != expected {
            return Err(io::Error::from(ErrorKind::InvalidData));
        }
    }
    Ok(())
}

fn quota_recovery() {
    const WAIT_RESOURCE: u64 = 2;
    let (status, limit) = native(48, WAIT_RESOURCE, 0);
    check(status == 0 && limit == 1, 600);
    let mut file = expect(
        OpenOptions::new().read(true).write(true).open("/probe"),
        601,
    );
    expect(file.seek(SeekFrom::Start(45)), 602);
    let word = Arc::new(AtomicU32::new(0));
    let child_word = word.clone();
    let child = expect(
        thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                while child_word.load(Ordering::Acquire) == 0 {
                    let (status, _) = native(58, child_word.as_ptr() as u64, 0);
                    match status {
                        0 => {}
                        -5 => thread::yield_now(),
                        error => return error,
                    }
                }
                0
            }),
        603,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (status, used) = native(48, WAIT_RESOURCE, 1);
        check(status == 0, 604);
        if used == 1 {
            break;
        }
        check(Instant::now() < deadline, 605);
        thread::yield_now();
    }
    let error = file.write_at(b"Q", 8180).unwrap_err();
    check(error.raw_os_error() == Some(49), 606);
    event(235, 1);
    word.store(1, Ordering::Release);
    let (status, woke) = native(60, word.as_ptr() as u64, 1);
    check(status == 0 && woke == 1, 607);
    check(child.join().is_ok_and(|status| status == 0), 608);
    let (status, used) = native(48, WAIT_RESOURCE, 1);
    check(status == 0 && used == 0, 609);
    let mut byte = [0xff];
    expect(file.read_exact_at(&mut byte, 8180), 610);
    check(byte == [0], 611);
    expect(file.write_all_at(b"R", 8180), 612);
    expect(file.read_exact_at(&mut byte, 8180), 613);
    check(byte == *b"R", 614);
    check(expect(file.stream_position(), 615) == 45, 616);
    event(236, 1);
}

fn native(number: u64, first: u64, second: u64) -> (i64, u64) {
    let mut status = number;
    let value;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") status,
            in("rdi") first,
            in("rsi") second,
            lateout("rdx") value,
            lateout("r8") _, lateout("r9") _, lateout("r10") _, lateout("r11") _,
            options(nostack),
        );
    }
    (status as i64, value)
}
