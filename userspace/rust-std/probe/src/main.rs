#![feature(restricted_std)]
#![feature(seek_stream_len)]

extern crate cryolite_allocator as _;

mod file_ops;
mod files;
mod random;

use std::alloc::{Layout, alloc, dealloc};
use std::cell::RefCell;
use std::os::unix::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

static TLS_DROPS: AtomicUsize = AtomicUsize::new(0);

struct TlsDrop;

impl Drop for TlsDrop {
    fn drop(&mut self) {
        TLS_DROPS.fetch_add(1, Ordering::Release);
    }
}

thread_local! {
    static TLS_PROBE: RefCell<Option<TlsDrop>> = const { RefCell::new(None) };
}

unsafe extern "C" {
    fn __cryolite_debug_handle() -> u64;
    fn __cryolite_thread_records() -> usize;
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
        exit(255);
    }
}

fn exit(code: u64) -> ! {
    unsafe {
        core::arch::asm!("int 0x80", "ud2", in("rax") 1u64, in("rdi") code, options(noreturn));
    }
}

fn check(condition: bool, stage: u64) {
    if !condition {
        event(229, stage);
        exit(stage);
    }
}

fn allocation() {
    let args: Vec<_> = std::env::args_os().collect();
    check(args.len() == 3 && args[1].as_bytes() == b"cryolite-v3", 1);
    check(matches!(args[2].as_bytes(), b"concurrent" | b"quota"), 1);
    let mut values = Vec::new();
    for value in 0..4096u64 {
        values.push(value);
    }
    check(values.iter().copied().sum::<u64>() == 8_386_560, 2);
    let text = format!("Cryolite {}", values.len());
    check(text == "Cryolite 4096", 3);
    check(values.try_reserve(usize::MAX).is_err(), 4);
    let mut limited = Vec::<u8>::new();
    check(limited.try_reserve_exact(64 * 1024 * 1024).is_err(), 7);
    let mut recovered = Vec::<u8>::new();
    check(recovered.try_reserve_exact(64 * 1024).is_ok(), 8);
    recovered.resize(64 * 1024, 0x5a);
    check(recovered[recovered.len() - 1] == 0x5a, 9);
    let layout = Layout::from_size_align(1024, 4096).unwrap();
    unsafe {
        let memory = alloc(layout);
        check(!memory.is_null() && memory.addr() % 4096 == 0, 5);
        memory.write_bytes(0xa5, 1024);
        check(memory.add(1023).read() == 0xa5, 6);
        dealloc(memory, layout);
    }
    event(220, 1);
}

fn synchronization() {
    let shared = Arc::new((Mutex::new((0u64, 0usize)), Condvar::new()));
    let guard = shared.0.lock().unwrap();
    let (guard, timeout) = shared
        .1
        .wait_timeout(guard, Duration::from_millis(2))
        .unwrap();
    check(timeout.timed_out() && guard.0 == 0, 10);
    let entered = Arc::new(AtomicUsize::new(0));
    let mut workers = Vec::new();
    event(225, 1);
    for _ in 0..2 {
        let shared = shared.clone();
        let entered = entered.clone();
        workers.push(
            thread::Builder::new()
                .stack_size(256 * 1024)
                .spawn(move || {
                    entered.fetch_add(1, Ordering::Release);
                    for _ in 0..1024 {
                        shared.0.lock().unwrap().0 += 1;
                        thread::yield_now();
                    }
                    shared.0.lock().unwrap().1 += 1;
                    shared.1.notify_all();
                })
                .unwrap(),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while entered.load(Ordering::Acquire) != 2 {
        check(Instant::now() < deadline, 13);
        thread::yield_now();
    }
    thread::sleep(Duration::from_millis(10));
    event(226, 1);
    drop(guard);
    let mut guard = shared.0.lock().unwrap();
    while guard.1 < 2 {
        guard = shared.1.wait(guard).unwrap();
    }
    check(guard.0 == 2048, 11);
    drop(guard);
    for worker in workers {
        check(worker.join().is_ok(), 12);
    }
    event(221, 1);
}

fn thread_lifecycle() {
    let joined = thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            TLS_PROBE.with(|slot| *slot.borrow_mut() = Some(TlsDrop));
            42u64
        })
        .unwrap();
    check(joined.join().unwrap() == 42, 20);
    check(TLS_DROPS.load(Ordering::Acquire) == 1, 21);
    event(222, 1);

    let release = Arc::new(AtomicBool::new(false));
    let child_release = release.clone();
    let detached = thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            TLS_PROBE.with(|slot| *slot.borrow_mut() = Some(TlsDrop));
            while !child_release.load(Ordering::Acquire) {
                thread::yield_now();
            }
        })
        .unwrap();
    drop(detached);
    release.store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(2);
    while TLS_DROPS.load(Ordering::Acquire) != 2 || unsafe { __cryolite_thread_records() } != 1 {
        check(Instant::now() < deadline, 22);
        thread::sleep(Duration::from_millis(1));
    }
    event(223, 1);
}

fn monotonic_time() {
    let start = Instant::now();
    thread::sleep(Duration::from_millis(2));
    check(start.elapsed() >= Duration::from_millis(2), 30);
    event(224, 1);
}

fn main() {
    std::panic::set_hook(Box::new(|_| {
        event(229, 250);
        exit(250);
    }));
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument.as_bytes() == b"abort")
    {
        event(227, 1);
        std::process::abort();
    }
    allocation();
    random::check_random();
    files::check_files();
    let concurrent = std::env::args_os()
        .nth(2)
        .is_some_and(|argument| argument.as_bytes() == b"concurrent");
    file_ops::check_file_ops(concurrent);
    synchronization();
    thread_lifecycle();
    monotonic_time();
}
