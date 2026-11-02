#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::hint::black_box;
use core::panic::PanicInfo;
use kryo_interfaces::syscall::Call;
use kryo_libsys::{self as sys, Handle, ObjectKind};

const STACK_BASE: u64 = 0x80_0000;
const STACK_END: u64 = STACK_BASE + 4 * 4096;

#[repr(C)]
pub struct Mixed {
    integer: u64,
    floating: f64,
}

global_asm!(
    r#"
    .section .text.start,"ax"
    .global _start
    .type _start,@function
_start:
    cld
    xor ebp, ebp
    mov rsi, rdi
    mov rdi, rsp
    call {main}
    ud2
    .size _start, .-_start
"#,
    main = sym probe_main,
);

unsafe extern "C" {
    fn c_mix(first: u64, second: f64, third: u64, fourth: f64) -> f64;
    fn c_register_pressure(
        a: f64,
        b: f64,
        c: f64,
        d: f64,
        e: f64,
        f: f64,
        g: f64,
        h: f64,
        ninth: f64,
        integer: u64,
    ) -> f64;
    fn c_calls_rust() -> u64;
    fn c_variadic(count: u32, ...) -> f64;
    fn cpp_mix(first: u64, second: f64, third: u64, fourth: f64) -> f64;
    fn cpp_aggregate(value: Mixed) -> Mixed;
    fn cpp_calls_rust() -> u64;
}

#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn rust_aggregate(value: Mixed) -> Mixed {
    Mixed {
        integer: value.integer + 7,
        floating: value.floating * 2.0,
    }
}

#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn rust_mix(first: u64, second: f64, third: u64, fourth: f64) -> f64 {
    let value = black_box(first as f64 + second * 2.0 - third as f64 + fourth * 4.0);
    checked_call(Call::Yield, 0, 0);
    black_box(value)
}

#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn rust_register_pressure(
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
    g: f64,
    h: f64,
    ninth: f64,
    integer: u64,
) -> f64 {
    a + b + c + d + e + f + g + h + ninth + integer as f64
}

extern "C" fn probe_main(stack: u64, startup: u64) -> ! {
    check_stack(stack);
    let debug = startup_debug(startup);
    for round in 0..32u64 {
        let first = black_box(19 + round);
        let second = black_box(1.25);
        let third = black_box(7);
        let fourth = black_box(0.5);
        if unsafe { c_mix(first, second, third, fourth) } != 16.5 + round as f64 {
            exit(201);
        }
        if unsafe { c_register_pressure(1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10) } != 55.0 {
            exit(202);
        }
        if unsafe { c_calls_rust() } != 0 {
            exit(203);
        }
        if unsafe { cpp_mix(first, second, third, fourth) } != 16.5 + round as f64
            || unsafe { cpp_calls_rust() } != 0
        {
            exit(209);
        }
        let aggregate = unsafe {
            cpp_aggregate(Mixed {
                integer: first,
                floating: second,
            })
        };
        if aggregate.integer != first + 7 || aggregate.floating != second * 2.0 {
            exit(210);
        }
        if unsafe {
            c_variadic(
                9, 1.0f64, 2.0f64, 3.0f64, 4.0f64, 5.0f64, 6.0f64, 7.0f64, 8.0f64, 9.0f64,
            )
        } != 45.0
        {
            exit(211);
        }
        let value = black_box(123.75 + round as f64);
        vector_call(Call::Yield, 0, round);
        let now = checked_call(Call::ClockRead, 0, 0);
        vector_call(
            Call::SleepUntil,
            now.checked_add(100_000).unwrap_or_else(|| exit(204)),
            round,
        );
        if black_box(value * 2.0) != 247.5 + round as f64 * 2.0 {
            exit(205);
        }
    }
    preempt_vector(debug);
    sys::debug_log(debug, 90, 2).unwrap_or_else(|_| exit(207));
    exit(0)
}

fn vector_call(number: Call, argument: u64, seed: u64) {
    let first = [seed, seed + 1, seed + 2, seed + 3];
    let last = [seed + 4, seed + 5, seed + 6, seed + 7];
    let mut output = [[0u64; 4]; 2];
    let status: u64;
    // YMM15 exercises the upper register bank. Native calls preserve full user
    // state; a C caller-saved register list would hide a broken syscall path.
    // Scratch outputs cannot overlap the output address used after the trap.
    unsafe {
        asm!("vmovdqu ymm0, [{first}]", "vmovdqu ymm15, [{last}]", "int 0x80",
            "vmovdqu [{output}], ymm0", "vmovdqu [{output} + 32], ymm15",
            first = in(reg) first.as_ptr(), last = in(reg) last.as_ptr(),
            output = in(reg) output.as_mut_ptr(),
            inlateout("rax") number as u64 => status,
            in("rdi") argument, in("rsi") 0u64, out("rdx") _,
            out("ymm0") _, out("ymm15") _,
            out("r8") _, out("r9") _, out("r10") _, out("r11") _, options(nostack));
    }
    if status != 0 || output != [first, last] {
        exit(208);
    }
}

fn preempt_vector(debug: Handle) {
    const ROUNDS: u64 = 2_000_000;
    let input = [11u64, 23, 37, 41];
    let delta = [1u64; 4];
    let mut output = [0u64; 4];
    let first_status: u64;
    let last_status: u64;
    // Both markers bracket live YMM state inside one assembly block. The
    // kernel must observe timer preemption between them; the loop makes no
    // calls. Early-clobber outputs retain every address used after a trap.
    unsafe {
        asm!("vmovdqu ymm15, [{input}]", "vmovdqu ymm0, [{delta}]",
            "int 0x80", "mov {first_status}, rax", "test rax, rax", "jnz 3f",
            "2:", "vpaddq ymm15, ymm15, ymm0", "dec rcx", "jnz 2b",
            "mov eax, {debug_log}", "mov r8d, 1", "int 0x80",
            "3:",
            "vmovdqu [{output}], ymm15",
            input = in(reg) input.as_ptr(), delta = in(reg) delta.as_ptr(),
            output = in(reg) output.as_mut_ptr(), first_status = out(reg) first_status,
            debug_log = const Call::DebugLog as u64,
            inlateout("rax") Call::DebugLog as u64 => last_status,
            in("rdi") debug.raw(), in("rsi") 90u64,
            inout("rcx") ROUNDS => _, inout("r8") 0u64 => _,
            out("rdx") _, out("r9") _, out("r10") _, out("r11") _,
            out("ymm0") _, out("ymm15") _, options(nostack));
    }
    if first_status != 0 || last_status != 0 {
        exit(207);
    }
    if output != input.map(|value| value + ROUNDS) {
        exit(212);
    }
}

fn checked_call(number: Call, first: u64, second: u64) -> u64 {
    let status: u64;
    let result: u64;
    // Native entry preserves this thread's XMM state. These are syscall
    // register effects, not the caller-saved set of a C function call.
    unsafe {
        asm!("int 0x80", inlateout("rax") number as u64 => status,
            in("rdi") first, in("rsi") second, lateout("rdx") result,
            lateout("r8") _, lateout("r9") _, lateout("r10") _, lateout("r11") _, options(nostack));
    }
    if status != 0 {
        exit(206);
    }
    result
}

fn exit(code: u64) -> ! {
    sys::exit(code)
}

fn startup_debug(raw: u64) -> Handle {
    let record = Handle::from_raw(raw).unwrap_or_else(|| exit(220));
    let startup = sys::startup(record).unwrap_or_else(|_| exit(220));
    if startup.service != 50
        || startup.instance != 1
        || startup.argument != 0
        || startup.grants().count() != 1
    {
        exit(220);
    }
    let debug = startup.grant(1).unwrap_or_else(|| exit(220));
    if debug.kind != ObjectKind::DebugConsole || debug.rights != sys::WRITE {
        exit(220);
    }
    let info = sys::handle_info(debug.handle).unwrap_or_else(|_| exit(220));
    if info.kind != debug.kind || info.rights != debug.rights {
        exit(220);
    }
    debug.handle
}

fn check_stack(stack: u64) {
    if !stack.is_multiple_of(16) || stack_word(stack, 0) != 2 {
        exit(221);
    }
    check_string(stack_word(stack, 1), b"abi-probe");
    check_string(stack_word(stack, 2), b"cryolite-v3");
    check_string(stack_word(stack, 4), b"KRYO_ABI=1");
    if stack_word(stack, 3) != 0 || stack_word(stack, 5) != 0 {
        exit(221);
    }
    let auxiliary = [6, 4096, 9, _start as *const () as u64, 0, 0];
    for (index, expected) in auxiliary.into_iter().enumerate() {
        if stack_word(stack, 6 + index) != expected {
            exit(221);
        }
    }
}

fn stack_word(stack: u64, index: usize) -> u64 {
    let address = stack
        .checked_add(index as u64 * 8)
        .unwrap_or_else(|| exit(221));
    if !(STACK_BASE..=STACK_END - 8).contains(&address) || !address.is_multiple_of(8) {
        exit(221);
    }
    // The fixture maps this entire initialized private stack before entry.
    unsafe { (address as *const u64).read() }
}

fn check_string(address: u64, expected: &[u8]) {
    let end = address
        .checked_add(expected.len() as u64)
        .unwrap_or_else(|| exit(221));
    if address < STACK_BASE || end >= STACK_END {
        exit(221);
    }
    for (index, expected) in expected.iter().copied().chain([0]).enumerate() {
        if unsafe { (address as *const u8).add(index).read() } != expected {
            exit(221);
        }
    }
}

unsafe extern "C" {
    fn _start();
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    exit(250)
}
