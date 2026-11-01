#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;
use kryo_libsys::{self as sys, Error, ExitReason, Grant, Handle, ProcessExit, TaskStatus, WRITE};

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
    main = sym init_main,
);

extern "C" fn init_main(image: u64, debug: u64, features: u64) -> ! {
    let debug = match Handle::from_raw(debug) {
        Some(handle) => handle,
        None => sys::exit(100),
    };
    let image = Handle::from_raw(image).unwrap_or_else(|| fail(debug, 101));
    if image == debug || features & !1 != 0 {
        fail(debug, 102);
    }
    sys::debug_log(debug, 1, 0).unwrap_or_else(|_| fail(debug, 103));
    match sys::spawn(debug, &[]) {
        Err(Error::Kernel(-3 | -4)) => {}
        _ => fail(debug, 104),
    }

    let grants = [Grant {
        handle: debug,
        rights: WRITE,
    }];
    let first = sys::spawn(image, &grants).unwrap_or_else(|_| fail(debug, 110));
    check_exit(sys::process_wait(first), debug, 111);
    sys::debug_log(debug, 2, 42).unwrap_or_else(|_| fail(debug, 112));

    let second = sys::spawn(image, &grants).unwrap_or_else(|_| fail(debug, 120));
    if first == second {
        fail(debug, 121);
    }
    check_exit(sys::process_wait(first), debug, 122);
    if sys::task_status(first) != Ok(TaskStatus::Exited as u64) {
        fail(debug, 141);
    }
    if sys::task_stop(first, 99) != Err(Error::Kernel(-7)) {
        fail(debug, 142);
    }
    if sys::cancel_wait(first) != Err(Error::Kernel(-7)) {
        fail(debug, 143);
    }
    check_exit(sys::process_wait(second), debug, 123);
    sys::debug_log(debug, 2, 42).unwrap_or_else(|_| fail(debug, 124));
    sys::close(first).unwrap_or_else(|_| fail(debug, 125));
    sys::close(second).unwrap_or_else(|_| fail(debug, 126));
    sys::debug_log(debug, 3, 2).unwrap_or_else(|_| fail(debug, 127));
    sys::sleep_until(u64::MAX).unwrap_or_else(|_| fail(debug, 130));
    fail(debug, 131)
}

fn check_exit(result: Result<ProcessExit, Error>, debug: Handle, code: u64) {
    let result = result.unwrap_or_else(|_| fail(debug, code));
    if result.code != 42 || result.reason != ExitReason::Exited {
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
