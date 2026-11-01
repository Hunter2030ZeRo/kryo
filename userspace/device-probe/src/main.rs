#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;
use kryo_libsys::{self as sys, Error, Handle};

const COUNTER: u64 = 0xf0;
const CONFIG: u64 = 0x120;
const COMPARATOR: u64 = 0x128;
const DISABLED: u32 = 0x4100;
const ENABLED: u32 = 0x4104;
const WAIT_NS: u64 = 200_000_000;

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

    .section .text.device_child_start,"ax"
    .global device_child_start
    .type device_child_start,@function
device_child_start:
    cld
    xor ebp, ebp
    call {child}
    ud2
    .size device_child_start, .-device_child_start
"#,
    main = sym probe_main,
    child = sym revocation_observer,
);

unsafe extern "C" {
    fn device_child_start() -> !;
}

extern "C" fn probe_main(device: u64, notification: u64, ticks: u64, mode: u64) -> ! {
    let device = Handle::from_raw(device).unwrap_or_else(|| sys::exit(200));
    let notification = Handle::from_raw(notification).unwrap_or_else(|| sys::exit(201));
    if device == notification || ticks == 0 || ticks > u64::from(u32::MAX / 2) || mode > 3 {
        sys::exit(202);
    }
    let result = probe(device, notification, ticks as u32, mode);
    if sys::device_write(device, CONFIG, DISABLED).is_err() {
        sys::exit(230);
    }
    if let Err(code) = result {
        sys::exit(code);
    }
    sys::close(device).unwrap_or_else(|_| sys::exit(231));
    if sys::device_read(device, COUNTER) != Err(Error::Kernel(-2)) {
        sys::exit(232);
    }
    sys::close(notification).unwrap_or_else(|_| sys::exit(233));
    sys::exit(0)
}

fn probe(device: Handle, notification: Handle, ticks: u32, mode: u64) -> Result<(), u64> {
    if !invalid(sys::device_read(device, 0x80)) {
        return Err(210);
    }
    if sys::device_write(device, COUNTER, 0) != Err(Error::Kernel(-3)) {
        return Err(211);
    }
    if !matches!(
        sys::device_read(notification, COUNTER),
        Err(Error::Kernel(-3 | -4))
    ) {
        return Err(212);
    }
    for offset in [0x10, 0x20, 0x121, 0x130] {
        if !invalid(sys::device_read(device, offset)) {
            return Err(213);
        }
    }
    if !invalid(sys::device_write(device, CONFIG, u32::MAX)) {
        return Err(214);
    }

    // Timed mode can retry a match missed during register mediation. Both modes
    // require a delivered notification; counter advancement is not sufficient.
    for _ in 0..4 {
        sys::device_write(device, CONFIG, DISABLED).map_err(|_| 220u64)?;
        let start = sys::device_read(device, COUNTER).map_err(|_| 221u64)?;
        sys::device_write(device, COMPARATOR, start.wrapping_add(ticks)).map_err(|_| 222u64)?;
        sys::device_write(device, CONFIG, ENABLED).map_err(|_| 223u64)?;
        if mode == 2 {
            loop {
                sys::yield_now().unwrap_or_else(|_| sys::exit(229));
            }
        }
        if mode == 3 {
            // The child retains the process after the IRQ consumer exits.
            unsafe { sys::thread_create(device_child_start, device.raw()) }
                .unwrap_or_else(|_| sys::exit(240));
            // Only Copy values remain here; hardware teardown belongs to the
            // kernel's consumer-retirement path, not this thread's cleanup.
            unsafe { sys::thread_exit(0) }
        }
        let deadline = if mode == 1 {
            Some(
                sys::clock()
                    .map_err(|_| 224u64)?
                    .checked_add(WAIT_NS)
                    .ok_or(225u64)?,
            )
        } else {
            None
        };
        loop {
            let result = match deadline {
                Some(deadline) => sys::wait_notification_until(notification, deadline),
                None => sys::wait_notification(notification),
            };
            match result {
                Ok(()) => {
                    let end = sys::device_read(device, COUNTER).map_err(|_| 226u64)?;
                    let elapsed = end.wrapping_sub(start);
                    if elapsed >= ticks && elapsed < 1 << 31 {
                        return Ok(());
                    }
                    // A wrap or older coalesced IRQ may arrive before this
                    // target. Keep waiting for the already armed comparator.
                }
                Err(Error::Kernel(-10)) => break,
                Err(_) => return Err(227),
            }
        }
    }
    Err(228)
}

fn invalid<T>(result: Result<T, Error>) -> bool {
    matches!(result, Err(Error::Kernel(-6)))
}

extern "C" fn revocation_observer(device: u64) -> ! {
    let device = Handle::from_raw(device).unwrap_or_else(|| sys::exit(241));
    let deadline = sys::clock()
        .unwrap_or_else(|_| sys::exit(242))
        .checked_add(WAIT_NS)
        .unwrap_or_else(|| sys::exit(243));
    loop {
        match sys::device_read(device, COUNTER) {
            Err(Error::Kernel(-7)) => sys::exit(0),
            Ok(_) => {}
            Err(_) => sys::exit(244),
        }
        if sys::clock().unwrap_or_else(|_| sys::exit(245)) >= deadline {
            sys::exit(246);
        }
        sys::yield_now().unwrap_or_else(|_| sys::exit(247));
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    sys::exit(250)
}
