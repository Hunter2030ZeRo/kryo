#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU64, Ordering};
use kryo_libsys::{self as sys, Grant, Handle};

const CHILDREN: usize = 6;
const THREADS: usize = 20;
const TRANSFER: u64 = 1024;
const MAIN_TLS: u64 = 0x90_0000;
const TLS_SEED: u64 = 0x7363_616c_6500_0000;
const SLEEP_NS: u64 = 60_000_000_000;

static DEBUG: AtomicU64 = AtomicU64::new(0);
static READY: AtomicU64 = AtomicU64::new(0);
static ARGUMENTS: AtomicU64 = AtomicU64::new(0);
static TLS: [AtomicU64; THREADS] = [const { AtomicU64::new(0) }; THREADS];

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

    .section .text,"ax"
    .global scale_thread_start
    .type scale_thread_start,@function
scale_thread_start:
    cld
    xor ebp, ebp
    call {thread}
    ud2
    .size scale_thread_start, .-scale_thread_start
"#,
    main = sym probe_main,
    thread = sym thread_main,
);

unsafe extern "C" {
    fn scale_thread_start() -> !;
}

extern "C" fn probe_main(first: u64, second: u64, features: u64) -> ! {
    let debug = Handle::from_raw(if second == 0 { first } else { second })
        .unwrap_or_else(|| sys::exit(200));
    DEBUG.store(debug.raw(), Ordering::Release);
    if features & !1 != 0 {
        fail(debug, 201);
    }
    if second != 0 {
        parent(debug, first)
    } else {
        child(debug)
    }
}

fn parent(debug: Handle, image: u64) -> ! {
    let image = Handle::from_raw(image).unwrap_or_else(|| fail(debug, 202));
    let mut controls = [None; CHILDREN];
    for (index, slot) in controls.iter_mut().enumerate() {
        *slot = Some(
            sys::spawn(
                image,
                &[Grant {
                    handle: debug,
                    rights: sys::WRITE | TRANSFER,
                }],
            )
            .unwrap_or_else(|_| fail(debug, 210 + index as u64)),
        );
    }
    // Keep every control live while the guest fixture inspects and stops them.
    for control in controls.into_iter().flatten() {
        if sys::task_status(control).is_err() {
            fail(debug, 220);
        }
    }
    sys::debug_log(debug, 81, CHILDREN as u64).unwrap_or_else(|_| fail(debug, 221));
    sleep(debug)
}

fn child(debug: Handle) -> ! {
    let mut controls = [None; THREADS];
    for (index, slot) in controls.iter_mut().enumerate() {
        *slot = Some(
            // All argument storage is static; concurrent accesses use atomics.
            unsafe { sys::thread_create(scale_thread_start, index as u64) }
                .unwrap_or_else(|_| fail(debug, 230 + index as u64)),
        );
    }
    while READY.load(Ordering::Acquire) != THREADS as u64 {
        sys::yield_now().unwrap_or_else(|_| fail(debug, 250));
    }
    if ARGUMENTS.load(Ordering::Acquire) != (1 << THREADS) - 1 {
        fail(debug, 251);
    }
    for (index, tls) in TLS.iter().enumerate() {
        let address = tls.load(Ordering::Acquire);
        if TLS[..index]
            .iter()
            .any(|other| other.load(Ordering::Acquire) == address)
        {
            fail(debug, 252);
        }
    }
    for control in controls.into_iter().flatten() {
        if sys::task_status(control).is_err() {
            fail(debug, 253);
        }
    }
    sys::debug_log(debug, 80, THREADS as u64).unwrap_or_else(|_| fail(debug, 254));
    sleep(debug)
}

extern "C" fn thread_main(argument: u64, tls: u64, features: u64) -> ! {
    let debug = Handle::from_raw(DEBUG.load(Ordering::Acquire)).unwrap_or_else(|| sys::exit(260));
    if argument >= THREADS as u64
        || tls == 0
        || tls == MAIN_TLS
        || tls >= 1 << 47
        || !tls.is_multiple_of(4096)
        || features & !1 != 0
    {
        fail(debug, 261);
    }
    let bit = 1 << argument;
    if ARGUMENTS.fetch_or(bit, Ordering::AcqRel) & bit != 0 {
        fail(debug, 262);
    }
    TLS[argument as usize].store(tls, Ordering::Release);
    let sentinel = TLS_SEED | argument;
    unsafe { (tls as *mut u64).write_volatile(sentinel) };
    check_tls(debug, sentinel);
    sys::yield_now().unwrap_or_else(|_| fail(debug, 263));
    check_tls(debug, sentinel);
    READY.fetch_add(1, Ordering::AcqRel);
    sleep(debug)
}

fn check_tls(debug: Handle, expected: u64) {
    let actual: u64;
    // Each thread's FS base must still select its private TLS page after entry.
    unsafe { asm!("mov {}, fs:[0]", out(reg) actual, options(nostack, readonly)) };
    if actual != expected {
        fail(debug, 264);
    }
}

fn sleep(debug: Handle) -> ! {
    loop {
        sys::sleep_for(SLEEP_NS).unwrap_or_else(|_| fail(debug, 265));
    }
}

fn fail(debug: Handle, code: u64) -> ! {
    let _ = sys::debug_log(debug, 255, code);
    sys::exit(code)
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    let debug = Handle::from_raw(DEBUG.load(Ordering::Acquire));
    if let Some(debug) = debug {
        let _ = sys::debug_log(debug, 255, 299);
    }
    sys::exit(299)
}
