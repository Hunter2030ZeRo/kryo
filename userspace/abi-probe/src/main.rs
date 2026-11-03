#![no_std]
#![no_main]
#![feature(thread_local)]

use core::arch::{asm, global_asm};
use core::cell::UnsafeCell;
use core::hint::black_box;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use kryo_interfaces::syscall::Call;
use kryo_libsys::{self as sys, Handle, ObjectKind};

const STACK_BASE: u64 = 0x80_0000;
const STACK_END: u64 = STACK_BASE + 4 * 4096;
const IMAGE_BASE: u64 = 0x40_0000;
const TLS_SLOT_BYTES: usize = 4096;
const TLS_THREADS: usize = 3;
const MAIN_TLS_VALUE: u64 = 0x1000;

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
"#,
    main = sym probe_main,
    thread = sym tls_thread,
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
