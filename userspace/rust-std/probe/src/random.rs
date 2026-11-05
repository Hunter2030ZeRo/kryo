use std::collections::HashMap;
use std::thread;

use super::{check, event};

const BUSY: i64 = -11;
const UNSUPPORTED: i64 = -12;
const EAGAIN: i32 = 6;
const ENOTSUP: i32 = 61;

unsafe extern "C" {
    fn __cryolite_rust_random(output: *mut u8, bytes: usize) -> i32;
}

fn words(first: u64, second: u64) -> (i64, u64, u64) {
    let mut status = 67u64;
    let mut low = u64::MAX;
    let mut high = u64::MAX;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inout("rax") status,
            in("rdi") first,
            in("rsi") second,
            inout("rdx") low,
            inout("r8") high,
            lateout("r9") _, lateout("r10") _, lateout("r11") _,
            options(nostack),
        );
    }
    (status as i64, low, high)
}

fn call(number: u64, first: u64, second: u64) -> (i64, u64) {
    let mut status = number;
    let value;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inout("rax") status,
            in("rdi") first,
            in("rsi") second,
            lateout("rdx") value,
            lateout("r8") _, lateout("r9") _, lateout("r10") _, lateout("r11") _,
            options(nostack),
        );
    }
    (status as i64, value)
}

fn native_source() -> bool {
    for (first, second) in [(1, 0), (0, 1), (u64::MAX, u64::MAX)] {
        check(words(first, second) == (-6, 0, 0), 500);
    }
    for _ in 0..16 {
        let (status, low, high) = words(0, 0);
        match status {
            0 => return true,
            BUSY => {
                check(low == 0 && high == 0, 501);
                thread::yield_now();
            }
            UNSUPPORTED => {
                check(low == 0 && high == 0, 502);
                return false;
            }
            _ => check(false, 503),
        }
    }
    check(false, 504);
    false
}

fn fill(output: &mut [u8]) {
    for _ in 0..16 {
        output.fill(0xa5);
        match unsafe { __cryolite_rust_random(output.as_mut_ptr(), output.len()) } {
            0 => return,
            EAGAIN => {
                check(output.iter().all(|&byte| byte == 0xa5), 505);
                thread::yield_now();
            }
            _ => check(false, 506),
        }
    }
    check(false, 507);
}

fn guarded_buffer(supported: bool) {
    let (status, base) = call(54, 8192, 4096);
    check(status == 0, 508);
    check(call(55, base, 4096).0 == 0, 509);
    // Reserve owns both pages. Only the first is committed; the next page is
    // inaccessible, so a 256-byte request must stop exactly at this boundary.
    unsafe {
        let output = (base as *mut u8).add(4096 - 256);
        output.sub(1).write(0x5a);
        let bytes = std::slice::from_raw_parts_mut(output, 256);
        bytes.fill(0xa5);
        if supported {
            fill(bytes);
        } else {
            check(__cryolite_rust_random(output, 256) == ENOTSUP, 510);
            check(bytes.iter().all(|&byte| byte == 0xa5), 511);
        }
        check(output.sub(1).read() == 0x5a, 512);
    }
    check(call(57, base, 0).0 == 0, 513);
}

pub(super) fn check_random() {
    let supported = native_source();
    check(
        unsafe { __cryolite_rust_random(std::ptr::null_mut(), 0) } == 0,
        514,
    );
    check(
        unsafe { __cryolite_rust_random(usize::MAX as *mut u8, 0) } == 0,
        522,
    );
    let mut bytes = [0xa5; 258];
    check(
        unsafe { __cryolite_rust_random(bytes.as_mut_ptr().add(1), 257) } == 28,
        515,
    );
    check(bytes.iter().all(|&byte| byte == 0xa5), 516);
    if supported {
        for size in [1, 15, 16, 17, 255, 256] {
            bytes.fill(0xa5);
            fill(&mut bytes[1..size + 1]);
            check(
                bytes[0] == 0xa5 && bytes[size + 1..].iter().all(|&byte| byte == 0xa5),
                517,
            );
        }
        let mut values = HashMap::new();
        for number in 0..128u64 {
            values.insert(number, number * 37 + 11);
        }
        check(
            values.len() == 128 && values.get(&42) == Some(&(42 * 37 + 11)),
            518,
        );
        check(
            values.remove(&42) == Some(42 * 37 + 11) && !values.contains_key(&42),
            519,
        );
    } else {
        check(
            unsafe { __cryolite_rust_random(bytes.as_mut_ptr().add(1), 256) } == ENOTSUP,
            520,
        );
        check(bytes.iter().all(|&byte| byte == 0xa5), 521);
    }
    guarded_buffer(supported);
    event(231, if supported { 1 } else { 2 });
}
