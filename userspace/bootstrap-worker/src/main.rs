#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU64, Ordering};
use kryo_libsys::{self as sys, Error, ExitReason, Handle, ProcessExit, TaskStatus};

const MAIN_TLS: u64 = 0x90_0000;
const TLS_SEED: u64 = 0x746c_7300_0000_0000;
static THREAD_HANDLE: AtomicU64 = AtomicU64::new(0);
static SHARED_RESULT: AtomicU64 = AtomicU64::new(0);
static DEBUG_HANDLE: AtomicU64 = AtomicU64::new(0);

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
    main = sym worker_main,
);
global_asm!(include_str!("threads.S"), main = sym thread_main);

unsafe extern "C" {
    fn thread_start() -> !;
    fn state_probe(seed: u64, tls_value: u64, features: u64) -> u64;
}

extern "C" fn worker_main(debug: u64, extra: u64, features: u64) -> ! {
    let debug = match Handle::from_raw(debug) {
        Some(handle) => handle,
        None => sys::exit(200),
    };
    if extra != 0 || features & !1 != 0 {
        fail(debug, 201);
    }
    sys::debug_log(debug, 4, 0).unwrap_or_else(|_| fail(debug, 202));
    DEBUG_HANDLE.store(debug.raw(), Ordering::Release);
    // The bootstrap loader retains this private page for the main thread.
    unsafe {
        sys::set_tls(MAIN_TLS).unwrap_or_else(|_| fail(debug, 203));
        (MAIN_TLS as *mut u64).write_volatile(TLS_SEED | 0x100);
    }
    let first = start_thread(debug, 7);
    probe_main(debug, features, 7);
    check_join(sys::thread_join(first), 7, debug, 210);
    if SHARED_RESULT.load(Ordering::Acquire) != 7 {
        fail(debug, 211);
    }

    let second = start_thread(debug, 8);
    probe_main(debug, features, 8);
    check_join(sys::thread_join(first), 7, debug, 212);
    if sys::task_stop(first, 99) != Err(Error::Kernel(-7)) {
        fail(debug, 213);
    }
    check_join(sys::thread_join(second), 8, debug, 214);
    if SHARED_RESULT.load(Ordering::Acquire) != 8 {
        fail(debug, 215);
    }
    // The child closed this process's handle, while init retains its own copy.
    if sys::debug_log(debug, 4, 1) != Err(Error::Kernel(-2)) {
        fail(debug, 216);
    }
    sys::close(first).unwrap_or_else(|_| fail(debug, 217));
    sys::close(second).unwrap_or_else(|_| fail(debug, 218));
    sys::exit(42)
}

fn start_thread(debug: Handle, code: u64) -> Handle {
    THREAD_HANDLE.store(0, Ordering::Release);
    SHARED_RESULT.store(0, Ordering::Release);
    // The argument is a number; shared storage is static and all accesses atomic.
    let thread =
        unsafe { sys::thread_create(thread_start, code) }.unwrap_or_else(|_| fail(debug, 204));
    THREAD_HANDLE.store(thread.raw(), Ordering::Release);
    thread
}

fn probe_main(debug: Handle, features: u64, child: u64) {
    let result = unsafe { state_probe(child ^ 1, TLS_SEED | 0x100, features) };
    if result != 1 {
        fail(debug, 205);
    }
}

extern "C" fn thread_main(code: u64, tls: u64, features: u64) -> ! {
    let debug =
        Handle::from_raw(DEBUG_HANDLE.load(Ordering::Acquire)).unwrap_or_else(|| sys::exit(220));
    if !matches!(code, 7 | 8)
        || tls == 0
        || tls == MAIN_TLS
        || tls >= 1 << 47
        || !tls.is_multiple_of(4096)
        || features & !1 != 0
    {
        fail(debug, 221);
    }
    let handle = loop {
        if let Some(handle) = Handle::from_raw(THREAD_HANDLE.load(Ordering::Acquire)) {
            break handle;
        }
        sys::yield_now().unwrap_or_else(|_| fail(debug, 222));
    };
    // This handle was installed after the child was published, so resolving it
    // also checks that both threads observe the same live handle table.
    let status = sys::task_status(handle).unwrap_or_else(|_| fail(debug, 223));
    if status > TaskStatus::Blocked as u64 {
        fail(debug, 224);
    }
    let sentinel = TLS_SEED | code;
    unsafe { (tls as *mut u64).write_volatile(sentinel) };
    if unsafe { state_probe(code, sentinel, features) } != 1 {
        fail(debug, 225);
    }
    SHARED_RESULT.store(code, Ordering::Release);
    if code == 8 {
        sys::close(debug).unwrap_or_else(|_| fail(debug, 226));
    }
    // No guard or owned allocation remains on the child stack.
    unsafe { sys::thread_exit(code) }
}

fn check_join(result: Result<ProcessExit, Error>, expected: u64, debug: Handle, code: u64) {
    let result = result.unwrap_or_else(|_| fail(debug, code));
    if result.code != expected || result.reason != ExitReason::Exited {
        fail(debug, code);
    }
}

fn fail(debug: Handle, code: u64) -> ! {
    let _ = sys::debug_log(debug, 255, code);
    sys::exit(code)
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    sys::exit(250)
}
