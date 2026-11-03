#![no_std]
#![no_main]
#![feature(thread_local)]

use core::arch::{asm, global_asm};
use core::cell::UnsafeCell;
use core::hint::black_box;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use kryo_interfaces::syscall::{Call, Error as CallError};
use kryo_libsys::{self as sys, Handle, ObjectKind};

const STACK_BASE: u64 = 0x80_0000;
const STACK_END: u64 = STACK_BASE + 4 * 4096;
const IMAGE_BASE: u64 = 0x40_0000;
const TLS_SLOT_BYTES: usize = 4096;
const TLS_THREADS: usize = 3;
const MAIN_TLS_VALUE: u64 = 0x1000;
const PAGE_BYTES: u64 = 4096;
const PRIVATE_PAGES: u64 = 1024;
const RESERVED_PAGES: u64 = 65_536;
const REGION_PAGES: u64 = RESERVED_PAGES / 2;
const COMMITTED_PAGES: u64 = 512;

#[repr(C, align(4096))]
struct TlsStorage(UnsafeCell<[[u8; TLS_SLOT_BYTES]; TLS_THREADS]>);

// Each live thread has one exclusive slot. The main thread clears child slots
// only after native join proves those threads cannot execute again.
unsafe impl Sync for TlsStorage {}

static TLS_STORAGE: TlsStorage = TlsStorage(UnsafeCell::new([[0; TLS_SLOT_BYTES]; TLS_THREADS]));
static TLS_TEMPLATE: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static C_TLS_ADDRESSES: [AtomicU64; TLS_THREADS] = [const { AtomicU64::new(0) }; TLS_THREADS];
static CPP_TLS_ADDRESSES: [AtomicU64; TLS_THREADS] = [const { AtomicU64::new(0) }; TLS_THREADS];
static RUST_TLS_ADDRESSES: [AtomicU64; TLS_THREADS] = [const { AtomicU64::new(0) }; TLS_THREADS];
static CHILD_READY: AtomicUsize = AtomicUsize::new(0);
static WAIT_WORD: AtomicU32 = AtomicU32::new(0);
static WORD_RESULTS: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];
static MANAGED_TLS: AtomicU64 = AtomicU64::new(0);

#[repr(C, align(64))]
struct RustTlsValue(u64);

#[thread_local]
static mut RUST_TLS_INITIALIZED: RustTlsValue = RustTlsValue(0xa55a_1234_6789_4321);
#[thread_local]
static mut RUST_TLS_ZERO: [u64; 4] = [0; 4];

#[derive(Clone, Copy)]
struct TlsTemplate {
    address: u64,
    file_bytes: usize,
    memory_bytes: usize,
    alignment: usize,
}

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

    .global tls_thread_start
    .type tls_thread_start,@function
tls_thread_start:
    cld
    xor ebp, ebp
    call {thread}
    ud2
    .size tls_thread_start, .-tls_thread_start

    .global word_thread_start
    .type word_thread_start,@function
word_thread_start:
    cld
    xor ebp, ebp
    call {word_thread}
    ud2
    .size word_thread_start, .-word_thread_start
"#,
    main = sym probe_main,
    thread = sym tls_thread,
    word_thread = sym word_thread,
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
    fn c_tls_initial() -> u64;
    fn c_tls_store(value: u64);
    fn c_tls_check(value: u64) -> u64;
    fn c_tls_address() -> u64;
    fn cpp_tls_initial() -> u64;
    fn cpp_tls_store(value: u64);
    fn cpp_tls_check(value: u64) -> u64;
    fn cpp_tls_address() -> u64;
    fn tls_thread_start() -> !;
    fn word_thread_start() -> !;
    static __ehdr_start: u8;
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
    let template = check_stack(stack);
    let debug = startup_debug(startup);
    publish_template(template);
    initialize_tls(0, template);
    check_initial_tls();
    store_tls(MAIN_TLS_VALUE);
    sys::debug_log(debug, 94, 1).unwrap_or_else(|_| exit(207));
    check_thread_tls(debug);
    check_private_vm(debug);
    check_atomic_wait(debug);
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

fn check_private_vm(debug: Handle) {
    use sys::{BudgetQuery, Resource};

    // Child threads have joined. This fixture exclusively owns each reservation,
    // creates no Rust references into it and ends volatile accesses before
    // every decommit or release below.
    if vm_result(sys::budget_self(Resource::PrivatePages, BudgetQuery::Limit)) != PRIVATE_PAGES
        || vm_result(sys::budget_self(
            Resource::ReservedPages,
            BudgetQuery::Limit,
        )) != RESERVED_PAGES
    {
        exit(232);
    }
    check_vm_usage(0, 0);
    expect_vm_error(
        sys::vm_reserve(PAGE_BYTES, PAGE_BYTES + 1),
        CallError::InvalidArgument,
    );

    let full = reserve_private(RESERVED_PAGES);
    check_vm_usage(0, RESERVED_PAGES);
    expect_vm_error(
        sys::vm_reserve(PAGE_BYTES, PAGE_BYTES),
        CallError::OutOfResources,
    );
    vm_result(unsafe { sys::vm_release(full) });
    check_vm_usage(0, 0);

    let base = reserve_private(REGION_PAGES);
    vm_result(sys::vm_commit(base, COMMITTED_PAGES * PAGE_BYTES));
    check_vm_usage(COMMITTED_PAGES, REGION_PAGES);
    // These ranges are private, committed and accessed only by this thread.
    unsafe {
        check_zero_pages(base, COMMITTED_PAGES);
        write_page_pattern(base, 0, COMMITTED_PAGES);
    }
    vm_result(sys::yield_now());
    vm_result(sys::sleep_for(100_000));
    unsafe { check_page_pattern(base, 0, COMMITTED_PAGES) };

    for result in [
        sys::vm_commit(base, PAGE_BYTES),
        sys::vm_commit(base + 1, PAGE_BYTES),
        sys::vm_commit(!(PAGE_BYTES - 1), 2 * PAGE_BYTES),
        sys::vm_commit(base - PAGE_BYTES, PAGE_BYTES),
        unsafe { sys::vm_release(base + PAGE_BYTES) },
    ] {
        expect_vm_error(result, CallError::InvalidArgument);
    }
    let extra = base + COMMITTED_PAGES * PAGE_BYTES;
    expect_vm_error(
        sys::vm_commit(extra, 513 * PAGE_BYTES),
        CallError::OutOfResources,
    );
    check_vm_usage(COMMITTED_PAGES, REGION_PAGES);
    unsafe { check_page_pattern(base, 0, COMMITTED_PAGES) };

    let middle = base + 128 * PAGE_BYTES;
    vm_result(unsafe { sys::vm_decommit(middle, 16 * PAGE_BYTES) });
    check_vm_usage(COMMITTED_PAGES - 16, REGION_PAGES);
    unsafe {
        check_page_pattern(base, 0, 128);
        check_page_pattern(base, 144, COMMITTED_PAGES - 144);
    }

    // Refunded credit admits the entire range rejected at quota admission.
    // Reusing it also checks that rejection left no conflicting mapping.
    vm_result(sys::vm_commit(extra, 513 * PAGE_BYTES));
    check_vm_usage(COMMITTED_PAGES - 16 + 513, REGION_PAGES);
    unsafe { check_zero_pages(extra, 513) };
    vm_result(unsafe { sys::vm_decommit(extra, 513 * PAGE_BYTES) });
    vm_result(sys::vm_commit(middle, 16 * PAGE_BYTES));
    check_vm_usage(COMMITTED_PAGES, REGION_PAGES);
    unsafe {
        check_zero_pages(middle, 16);
        check_page_pattern(base, 0, 128);
        check_page_pattern(base, 144, COMMITTED_PAGES - 144);
        write_page_pattern(base, 128, 16);
    }

    let second = reserve_private(REGION_PAGES);
    check_vm_usage(COMMITTED_PAGES, RESERVED_PAGES);
    expect_vm_error(
        sys::vm_reserve(PAGE_BYTES, PAGE_BYTES),
        CallError::OutOfResources,
    );
    vm_result(sys::vm_commit(second, 12 * PAGE_BYTES));
    check_vm_usage(COMMITTED_PAGES + 12, RESERVED_PAGES);
    unsafe {
        check_zero_pages(second, 12);
        write_page_pattern(second, 0, 12);
        check_page_pattern(base, 0, COMMITTED_PAGES);
        check_page_pattern(second, 0, 12);
    }
    vm_result(unsafe { sys::vm_release(second) });
    check_vm_usage(COMMITTED_PAGES, REGION_PAGES);
    vm_result(unsafe { sys::vm_release(base) });
    check_vm_usage(0, 0);

    let reused = reserve_private(REGION_PAGES);
    if reused != base {
        exit(236);
    }
    vm_result(sys::vm_commit(reused, 12 * PAGE_BYTES));
    unsafe { check_zero_pages(reused, 12) };
    vm_result(unsafe { sys::vm_release(reused) });
    check_vm_usage(0, 0);

    // Leave both owners live so process teardown must retire backing and quota.
    let retained = reserve_private(8);
    vm_result(sys::vm_commit(retained, 4 * PAGE_BYTES));
    unsafe { check_zero_pages(retained, 4) };
    check_vm_usage(4, 8);
    check_tls(MAIN_TLS_VALUE);
    sys::debug_log(debug, 95, 1).unwrap_or_else(|_| exit(237));
}

fn check_atomic_wait(debug: Handle) {
    let address = core::ptr::addr_of!(WAIT_WORD) as u64;
    WAIT_WORD.store(1, Ordering::Release);
    if vm_result(sys::wake(address, u64::MAX)) != 0
        || vm_result(sys::park(address, 0)) != sys::ParkResult::ValueChanged
    {
        exit(238);
    }
    check_wait_usage(0);
    for invalid in [0, address + 1, IMAGE_BASE] {
        expect_vm_error(sys::park(invalid, 0), CallError::InvalidArgument);
        expect_vm_error(sys::wake(invalid, 1), CallError::InvalidArgument);
    }
    WAIT_WORD.store(0, Ordering::Release);
    let deadline = vm_result(sys::clock()) + 5_000_000;
    expect_vm_error(sys::park_until(address, 0, deadline), CallError::TimedOut);
    if vm_result(sys::clock()) < deadline {
        exit(238);
    }
    check_wait_usage(0);

    reset_word_results();
    let first = start_word(0, address, false);
    await_waits(1);
    let second = start_word(1, address, false);
    await_waits(2);
    WAIT_WORD.store(1, Ordering::Release);
    if vm_result(sys::wake(address, 1)) != 1 {
        exit(238);
    }
    finish_word(first, 0, 1);
    if WORD_RESULTS[1].load(Ordering::Acquire) != 0 || vm_result(sys::wake(address, 0)) != 0 {
        exit(238);
    }
    check_wait_usage(1);
    if vm_result(sys::wake(address, u64::MAX)) != 1 {
        exit(238);
    }
    finish_word(second, 1, 1);

    reset_word_results();
    WAIT_WORD.store(0, Ordering::Release);
    let cancelled = start_word(0, address, false);
    await_waits(1);
    if vm_result(sys::cancel_wait(cancelled)) {
        exit(238);
    }
    finish_word(cancelled, 0, 4);

    // No Rust reference points into this arena. Children pass only a numeric
    // address to the checked kernel call and never access it after cancellation.
    let arena = reserve_private(2);
    vm_result(sys::vm_commit(arena, PAGE_BYTES));
    expect_vm_error(sys::park(arena + PAGE_BYTES, 0), CallError::InvalidArgument);
    reset_word_results();
    let old = start_word(0, arena, false);
    await_waits(1);
    vm_result(unsafe { sys::vm_decommit(arena, PAGE_BYTES) });
    vm_result(sys::vm_commit(arena, PAGE_BYTES));
    if vm_result(sys::wake(arena, u64::MAX)) != 0 {
        exit(238);
    }
    let fresh = start_word(1, arena, false);
    await_waits(1);
    if vm_result(sys::wake(arena, 1)) != 1 {
        exit(238);
    }
    finish_word(old, 0, 4);
    finish_word(fresh, 1, 1);

    reset_word_results();
    let released = start_word(0, arena, false);
    await_waits(1);
    vm_result(unsafe { sys::vm_release(arena) });
    finish_word(released, 0, 4);
    let reused = reserve_private(2);
    if reused != arena {
        exit(238);
    }
    vm_result(sys::vm_commit(reused, PAGE_BYTES));
    if vm_result(sys::wake(reused, u64::MAX)) != 0 {
        exit(238);
    }
    vm_result(unsafe { sys::vm_release(reused) });

    reset_word_results();
    WAIT_WORD.store(0, Ordering::Release);
    let owner = start_word(0, address, true);
    await_waits(1);
    let tls = MANAGED_TLS.load(Ordering::Acquire);
    if tls == 0 {
        exit(238);
    }
    let observer = start_word(1, tls, false);
    await_waits(2);
    WAIT_WORD.store(1, Ordering::Release);
    if vm_result(sys::wake(address, 1)) != 1 {
        exit(238);
    }
    finish_word(owner, 0, 1);
    finish_word(observer, 1, 4);
    check_wait_usage(0);
    check_vm_usage(4, 8);
    check_tls(MAIN_TLS_VALUE);
    vm_result(sys::debug_log(debug, 96, 1));
}

fn reset_word_results() {
    for result in &WORD_RESULTS {
        result.store(0, Ordering::Release);
    }
}

fn start_word(index: u64, address: u64, publishes_tls: bool) -> Handle {
    let argument = address | index | (u64::from(publishes_tls) << 1);
    // The assembly entry implements the raw thread ABI; static atomics outlive
    // all children, and arena backing remains valid until the tested revocation.
    vm_result(unsafe { sys::thread_create(word_thread_start, argument) })
}

extern "C" fn word_thread(argument: u64, tls: u64) -> ! {
    let index = (argument & 1) as usize;
    if argument & 2 != 0 {
        MANAGED_TLS.store(tls, Ordering::Release);
    }
    let parked = if index == 0 {
        sys::park(argument & !3, 0)
    } else {
        sys::park_until(argument & !3, 0, vm_result(sys::clock()) + 1_000_000_000)
    };
    let result = match parked {
        Ok(sys::ParkResult::Woken) => 1,
        Ok(sys::ParkResult::ValueChanged) => 2,
        Err(sys::Error::Kernel(status)) if status == CallError::Cancelled.status() as i64 => 4,
        _ => exit(238),
    };
    WORD_RESULTS[index].store(result, Ordering::Release);
    unsafe { sys::thread_exit(0) }
}

fn finish_word(thread: Handle, index: usize, expected: usize) {
    let status = vm_result(sys::thread_join(thread));
    if status.reason != sys::ExitReason::Exited
        || status.code != 0
        || WORD_RESULTS[index].load(Ordering::Acquire) != expected
    {
        exit(238);
    }
    vm_result(sys::close(thread));
}

fn await_waits(expected: u64) {
    let deadline = vm_result(sys::clock()) + 1_000_000_000;
    loop {
        let used = vm_result(sys::budget_self(
            sys::Resource::Waits,
            sys::BudgetQuery::Used,
        ));
        if used == expected {
            return;
        }
        if vm_result(sys::clock()) >= deadline {
            exit(239);
        }
        vm_result(sys::yield_now());
    }
}

fn check_wait_usage(expected: u64) {
    if vm_result(sys::budget_self(
        sys::Resource::Waits,
        sys::BudgetQuery::Used,
    )) != expected
    {
        exit(238);
    }
}

fn vm_result<T>(result: Result<T, sys::Error>) -> T {
    result.unwrap_or_else(|_| exit(230))
}

fn expect_vm_error<T>(result: Result<T, sys::Error>, error: CallError) {
    if !matches!(result, Err(sys::Error::Kernel(status)) if status == error.status() as i64) {
        exit(231);
    }
}

fn check_vm_usage(private: u64, reserved: u64) {
    use sys::{BudgetQuery, Resource};

    if vm_result(sys::budget_self(Resource::PrivatePages, BudgetQuery::Used)) != private
        || vm_result(sys::budget_self(Resource::ReservedPages, BudgetQuery::Used)) != reserved
    {
        exit(232);
    }
}

fn reserve_private(pages: u64) -> u64 {
    let bytes = pages * PAGE_BYTES;
    let address = vm_result(sys::vm_reserve(bytes, PAGE_BYTES));
    if address < 0x1_0000_0000
        || !address.is_multiple_of(PAGE_BYTES)
        || address.checked_add(bytes).is_none()
    {
        exit(233);
    }
    address
}

// Callers retain exclusive access to all sampled committed pages. Decommit and
// release occur only after these volatile accesses end.
unsafe fn check_zero_pages(base: u64, pages: u64) {
    for page in 0..pages {
        let first = (base + page * PAGE_BYTES) as *const u64;
        let last = (base + (page + 1) * PAGE_BYTES - 8) as *const u64;
        if unsafe { first.read_volatile() } != 0 || unsafe { last.read_volatile() } != 0 {
            exit(234);
        }
    }
}

unsafe fn write_page_pattern(base: u64, first_page: u64, pages: u64) {
    for page in first_page..first_page + pages {
        let first = (base + page * PAGE_BYTES) as *mut u64;
        let last = (base + (page + 1) * PAGE_BYTES - 8) as *mut u64;
        let value = page_pattern(base, page);
        unsafe {
            first.write_volatile(value);
            last.write_volatile(!value);
        }
    }
}

unsafe fn check_page_pattern(base: u64, first_page: u64, pages: u64) {
    for page in first_page..first_page + pages {
        let first = (base + page * PAGE_BYTES) as *const u64;
        let last = (base + (page + 1) * PAGE_BYTES - 8) as *const u64;
        let value = page_pattern(base, page);
        if unsafe { first.read_volatile() } != value || unsafe { last.read_volatile() } != !value {
            exit(235);
        }
    }
}

fn page_pattern(base: u64, page: u64) -> u64 {
    0x6b72_796f_a55a_1234 ^ base.rotate_left(17) ^ page.wrapping_mul(0x9e37_79b9_7f4a_7c15)
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

fn publish_template(template: TlsTemplate) {
    TLS_TEMPLATE[1].store(template.file_bytes as u64, Ordering::Relaxed);
    TLS_TEMPLATE[2].store(template.memory_bytes as u64, Ordering::Relaxed);
    TLS_TEMPLATE[3].store(template.alignment as u64, Ordering::Relaxed);
    TLS_TEMPLATE[0].store(template.address, Ordering::Release);
}

fn template() -> TlsTemplate {
    let address = TLS_TEMPLATE[0].load(Ordering::Acquire);
    if address == 0 {
        exit(222);
    }
    TlsTemplate {
        address,
        file_bytes: TLS_TEMPLATE[1].load(Ordering::Relaxed) as usize,
        memory_bytes: TLS_TEMPLATE[2].load(Ordering::Relaxed) as usize,
        alignment: TLS_TEMPLATE[3].load(Ordering::Relaxed) as usize,
    }
}

fn slot(index: usize) -> *mut u8 {
    if index >= TLS_THREADS {
        exit(222);
    }
    TLS_STORAGE
        .0
        .get()
        .cast::<u8>()
        .wrapping_add(index * TLS_SLOT_BYTES)
}

fn initialize_tls(index: usize, template: TlsTemplate) {
    let length = template
        .memory_bytes
        .checked_add(template.alignment - 1)
        .map(|size| size & !(template.alignment - 1))
        .filter(|size| *size <= TLS_SLOT_BYTES - 8)
        .unwrap_or_else(|| exit(222));
    let storage = slot(index);
    let pointer = storage.wrapping_add(length);
    // The fixture uses aligned PT_TLS and minimal FS:0 self-pointer TCBs.
    // These fixed private slots do not implement mlibc's complete TCB layout.
    unsafe {
        storage.write_bytes(0, TLS_SLOT_BYTES);
        core::ptr::copy_nonoverlapping(template.address as *const u8, storage, template.file_bytes);
        pointer.cast::<u64>().write(pointer as u64);
        sys::set_tls(pointer as u64).unwrap_or_else(|_| exit(222));
    }
    let self_pointer: u64;
    unsafe {
        asm!("mov {}, qword ptr fs:[0]", out(reg) self_pointer, options(nostack, readonly, preserves_flags))
    };
    if self_pointer != pointer as u64 {
        exit(222);
    }
    let c_address = unsafe { c_tls_address() };
    let cpp_address = unsafe { cpp_tls_address() };
    let rust_address = rust_tls_address();
    let start = storage as u64;
    let end = pointer as u64;
    if !(start..end).contains(&c_address)
        || !(start..end).contains(&cpp_address)
        || !(start..end).contains(&rust_address)
    {
        exit(222);
    }
    C_TLS_ADDRESSES[index].store(c_address, Ordering::Release);
    CPP_TLS_ADDRESSES[index].store(cpp_address, Ordering::Release);
    RUST_TLS_ADDRESSES[index].store(rust_address, Ordering::Release);
}

#[inline(never)]
fn rust_tls_address() -> u64 {
    core::ptr::addr_of!(RUST_TLS_INITIALIZED) as u64
}

#[inline(never)]
fn rust_tls_initial() -> bool {
    if !rust_tls_address().is_multiple_of(64)
        || unsafe { core::ptr::addr_of!(RUST_TLS_INITIALIZED.0).read_volatile() }
            != 0xa55a_1234_6789_4321
    {
        return false;
    }
    for index in 0..4 {
        if unsafe {
            core::ptr::addr_of!(RUST_TLS_ZERO)
                .cast::<u64>()
                .add(index)
                .read_volatile()
        } != 0
        {
            return false;
        }
    }
    true
}

#[inline(never)]
fn rust_tls_store(value: u64) {
    unsafe {
        core::ptr::addr_of_mut!(RUST_TLS_INITIALIZED.0).write_volatile(value);
        for index in 0..4 {
            core::ptr::addr_of_mut!(RUST_TLS_ZERO)
                .cast::<u64>()
                .add(index)
                .write_volatile(value + index as u64 + 1);
        }
    }
}

#[inline(never)]
fn rust_tls_check(value: u64) -> bool {
    if unsafe { core::ptr::addr_of!(RUST_TLS_INITIALIZED.0).read_volatile() } != value {
        return false;
    }
    for index in 0..4 {
        if unsafe {
            core::ptr::addr_of!(RUST_TLS_ZERO)
                .cast::<u64>()
                .add(index)
                .read_volatile()
        } != value + index as u64 + 1
        {
            return false;
        }
    }
    true
}

fn check_initial_tls() {
    if unsafe { c_tls_initial() } != 0 || unsafe { cpp_tls_initial() } != 0 || !rust_tls_initial() {
        exit(223);
    }
}

fn store_tls(value: u64) {
    unsafe {
        c_tls_store(value);
        cpp_tls_store(value + 0x100);
    }
    rust_tls_store(value + 0x200);
}

fn check_tls(value: u64) {
    if unsafe { c_tls_check(value) } != 0
        || unsafe { cpp_tls_check(value + 0x100) } != 0
        || !rust_tls_check(value + 0x200)
    {
        exit(224);
    }
}

extern "C" fn tls_thread(index: u64) -> ! {
    if !(1..TLS_THREADS as u64).contains(&index) {
        exit(225);
    }
    initialize_tls(index as usize, template());
    check_initial_tls();
    let value = MAIN_TLS_VALUE + index;
    store_tls(value);
    CHILD_READY.fetch_add(1, Ordering::Release);
    for _ in 0..8 {
        sys::yield_now().unwrap_or_else(|_| exit(225));
        sys::sleep_for(100_000).unwrap_or_else(|_| exit(225));
        check_tls(value);
    }
    // The POD TLS fixture owns no destructors. The joiner clears this slot only
    // after exit has ended every execution use of its FS base.
    unsafe { sys::thread_exit(0) }
}

fn check_thread_tls(debug: Handle) {
    let first = unsafe { sys::thread_create(tls_thread_start, 1) }.unwrap_or_else(|_| exit(225));
    let second = unsafe { sys::thread_create(tls_thread_start, 2) }.unwrap_or_else(|_| exit(225));
    while CHILD_READY.load(Ordering::Acquire) != 2 {
        sys::yield_now().unwrap_or_else(|_| exit(225));
        check_tls(MAIN_TLS_VALUE);
    }
    for (index, thread) in [(1, first), (2, second)] {
        let status = sys::thread_join(thread).unwrap_or_else(|_| exit(225));
        if status.reason != sys::ExitReason::Exited || status.code != 0 {
            exit(225);
        }
        sys::close(thread).unwrap_or_else(|_| exit(225));
        // Native join has ended execution; clearing the dead thread's private
        // backing now cannot race its TLS loads or stores.
        unsafe { slot(index).write_bytes(0, TLS_SLOT_BYTES) };
        for offset in 0..TLS_SLOT_BYTES {
            if unsafe { slot(index).add(offset).read_volatile() } != 0 {
                exit(225);
            }
        }
    }
    for addresses in [&C_TLS_ADDRESSES, &CPP_TLS_ADDRESSES, &RUST_TLS_ADDRESSES] {
        for index in 0..TLS_THREADS {
            let address = addresses[index].load(Ordering::Acquire);
            if address == 0
                || addresses[..index]
                    .iter()
                    .any(|earlier| earlier.load(Ordering::Acquire) == address)
            {
                exit(225);
            }
        }
    }
    check_tls(MAIN_TLS_VALUE);
    sys::debug_log(debug, 94, 2).unwrap_or_else(|_| exit(207));
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

fn check_stack(stack: u64) -> TlsTemplate {
    if !stack.is_multiple_of(16) || stack_word(stack, 0) != 2 {
        exit(221);
    }
    check_string(stack_word(stack, 1), b"abi-probe");
    check_string(stack_word(stack, 2), b"cryolite-v3");
    check_string(stack_word(stack, 4), b"KRYO_ABI=1");
    if stack_word(stack, 3) != 0 || stack_word(stack, 5) != 0 {
        exit(221);
    }
    let mut values = [0; 5];
    let mut found = 0u8;
    let mut terminated = false;
    for index in 0..16 {
        let kind = stack_word(stack, 6 + index * 2);
        let value = stack_word(stack, 7 + index * 2);
        if kind == 0 {
            if value != 0 {
                exit(221);
            }
            terminated = true;
            break;
        }
        let slot = match kind {
            3 => 0,
            4 => 1,
            5 => 2,
            6 => 3,
            9 => 4,
            _ => exit(221),
        };
        if found & (1 << slot) != 0 {
            exit(221);
        }
        found |= 1 << slot;
        values[slot] = value;
    }
    if !terminated || found != 0x1f || values[3] != 4096 || values[4] != _start as *const () as u64
    {
        exit(221);
    }
    check_headers(values[0], values[1], values[2], values[4])
}

fn image_number(address: u64, bytes: usize) -> u64 {
    let end = address
        .checked_add(bytes as u64)
        .unwrap_or_else(|| exit(226));
    if address < IMAGE_BASE || end > STACK_BASE {
        exit(226);
    }
    let mut value = 0;
    for index in 0..bytes {
        // The fixture's loader validates these retained file-backed mappings.
        value |= u64::from(unsafe { (address as *const u8).add(index).read() }) << (index * 8);
    }
    value
}

fn check_headers(phdr: u64, entry_bytes: u64, count: u64, entry: u64) -> TlsTemplate {
    let header = core::ptr::addr_of!(__ehdr_start) as u64;
    if image_number(header, 8) != 0x0001_0102_464c_457f
        || image_number(header + 16, 2) != 2
        || image_number(header + 18, 2) != 62
        || image_number(header + 20, 4) != 1
        || image_number(header + 24, 8) != entry
        || image_number(header + 52, 2) != 64
        || entry_bytes != 56
        || image_number(header + 54, 2) != entry_bytes
        || image_number(header + 56, 2) != count
        || !(1..=64).contains(&count)
    {
        exit(226);
    }
    let offset = image_number(header + 32, 8);
    let bytes = count * entry_bytes;
    let header_end = offset
        .checked_add(bytes)
        .filter(|end| *end <= 4096)
        .unwrap_or_else(|| exit(226));
    if offset < 64 || header.checked_add(offset) != Some(phdr) {
        exit(226);
    }
    let mut headers_mapped = false;
    let mut phdr_mapped = false;
    let mut template = None;
    for index in 0..count {
        let pointer = phdr + index * entry_bytes;
        let kind = image_number(pointer, 4);
        let flags = image_number(pointer + 4, 4);
        let file_offset = image_number(pointer + 8, 8);
        let address = image_number(pointer + 16, 8);
        let file_bytes = image_number(pointer + 32, 8);
        let memory_bytes = image_number(pointer + 40, 8);
        let alignment = image_number(pointer + 48, 8);
        if kind == 1
            && file_offset == 0
            && address == header
            && file_bytes >= header_end
            && flags == 4
        {
            headers_mapped = true;
        }
        if kind == 6 {
            if phdr_mapped
                || file_offset != offset
                || address != phdr
                || file_bytes != bytes
                || memory_bytes != bytes
            {
                exit(226);
            }
            phdr_mapped = true;
        }
        if kind == 7 {
            if template.is_some()
                || file_bytes > memory_bytes
                || memory_bytes == 0
                || !(16..=TLS_SLOT_BYTES as u64).contains(&alignment)
                || !alignment.is_power_of_two()
                || !address.is_multiple_of(alignment)
                || address < IMAGE_BASE
                || address
                    .checked_add(memory_bytes)
                    .is_none_or(|end| end > STACK_BASE)
            {
                exit(226);
            }
            template = Some(TlsTemplate {
                address,
                file_bytes: file_bytes as usize,
                memory_bytes: memory_bytes as usize,
                alignment: alignment as usize,
            });
        }
    }
    if !headers_mapped || !phdr_mapped {
        exit(226);
    }
    template.unwrap_or_else(|| exit(226))
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
