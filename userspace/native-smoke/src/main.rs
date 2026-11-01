#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

const DATA_SEED: u64 = 0x1357_9bdf_2468_ace0;
const TLS_SEED: u64 = 0x746c_7300_0000_0000;

#[used]
static READ_ONLY: [u8; 64] = [0x53; 64];
static mut INITIALIZED: u64 = DATA_SEED;
static mut ZEROED: [u64; 512] = [0; 512];

#[unsafe(no_mangle)]
static mut native_smoke_result: u64 = 0;

global_asm!(
    r#"
    .section .text.start,"ax"
    .global _start
    .type _start,@function
_start:
    cld
    xor ebp, ebp
    call native_main
    ud2
    .size _start, .-_start

    .section .text.smoke_probe,"ax"
    .global smoke_probe
    .type smoke_probe,@function
smoke_probe:
    sub rsp, 48
    mov [rsp], rdi
    mov [rsp + 32], rdx
    fnstcw [rsp + 8]
    stmxcsr [rsp + 12]
    mov word ptr [rsp + 10], 0x077f
    mov dword ptr [rsp + 16], 0x3f80
    test dil, 1
    jnz 1f
    mov word ptr [rsp + 10], 0x0b7f
    mov dword ptr [rsp + 16], 0x5f80
1:
    fldcw [rsp + 10]
    ldmxcsr [rsp + 16]
    fild qword ptr [rsp]
    movq xmm15, rdi
    test qword ptr [rsp + 32], 1
    jz 8f
    vxorps ymm14, ymm14, ymm14
    vinsertf128 ymm14, ymm14, xmm15, 1
8:
    mov r8, rdi
    mov r9, rsi
    mov r10d, 2
2:
    xor eax, eax
    int 0x80
    test rax, rax
    jnz 5f
    mov r11d, 50000000
3:
    add rax, 1
    dec r11
    jnz 3b
    cmp rax, 50000000
    jne 5f
    movq rax, xmm15
    cmp rax, r8
    jne 5f
    test qword ptr [rsp + 32], 1
    jz 9f
    vextractf128 xmm13, ymm14, 1
    movq rax, xmm13
    cmp rax, r8
    jne 5f
9:
    cmp qword ptr fs:[0], r9
    jne 5f
    fistp qword ptr [rsp + 24]
    cmp [rsp + 24], r8
    jne 6f
    fild qword ptr [rsp]
    fnstcw [rsp + 24]
    mov ax, [rsp + 24]
    cmp ax, [rsp + 10]
    jne 5f
    stmxcsr [rsp + 24]
    mov eax, [rsp + 24]
    cmp eax, [rsp + 16]
    jne 5f
    dec r10
    jnz 2b
    fistp qword ptr [rsp + 24]
    movq rax, xmm15
    xor rax, r9
    add rax, r8
    jmp 7f
5:
    fstp st(0)
6:
    xor eax, eax
7:
    test qword ptr [rsp + 32], 1
    jz 10f
    vzeroupper
10:
    fldcw [rsp + 8]
    ldmxcsr [rsp + 12]
    add rsp, 48
    ret
    .size smoke_probe, .-smoke_probe
"#
);

unsafe extern "C" {
    fn smoke_probe(instance: u64, tls_value: u64, features: u64) -> u64;
}

#[unsafe(no_mangle)]
extern "C" fn native_main(instance: u64, tls_base: u64, features: u64) -> ! {
    if instance != 41 && instance != 42 {
        exit(100);
    }
    let data = addr_of_mut!(INITIALIZED);
    if unsafe { read_volatile(data) } != DATA_SEED {
        exit(101);
    }
    unsafe { write_volatile(data, DATA_SEED ^ instance) };
    for index in 0..64 {
        if unsafe { read_volatile(addr_of!(READ_ONLY).cast::<u8>().add(index)) } != 0x53 {
            exit(102);
        }
    }
    let zeroed = addr_of_mut!(ZEROED).cast::<u64>();
    let mut checksum = 0u64;
    for index in 0..512 {
        if unsafe { read_volatile(zeroed.add(index)) } != 0 {
            exit(103);
        }
        let value = instance.rotate_left((index % 64) as u32) ^ index as u64;
        unsafe { write_volatile(zeroed.add(index), value) };
        checksum = checksum.wrapping_add(value);
    }
    let invalid_argument = (-6i64) as u64;
    if set_tls(1 << 47) != invalid_argument || set_tls(tls_base) != 0 || tls_read() != 0 {
        exit(104);
    }
    let tls_value = TLS_SEED | instance;
    tls_write(tls_value);
    if set_tls(1 << 47) != invalid_argument || tls_read() != tls_value {
        exit(104);
    }
    // The scalar target never allocates SIMD registers. The assembly probe owns
    // its complete FP state until it returns, including across native yields.
    let probe = unsafe { smoke_probe(instance, tls_value, features) };
    if probe != (instance ^ tls_value).wrapping_add(instance) || tls_read() != tls_value {
        exit(105);
    }
    if unsafe { read_volatile(data) } != DATA_SEED ^ instance {
        exit(106);
    }
    let mut observed = 0u64;
    for index in 0..512 {
        observed = observed.wrapping_add(unsafe { read_volatile(zeroed.add(index)) });
    }
    if observed != checksum {
        exit(107);
    }
    unsafe { write_volatile(addr_of_mut!(native_smoke_result), probe ^ checksum) };
    exit(instance)
}

fn set_tls(base: u64) -> u64 {
    let status: u64;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") 19u64 => status,
            in("rdi") base,
            lateout("rdx") _,
            options(nostack),
        );
    }
    status
}

fn tls_read() -> u64 {
    let value;
    unsafe { asm!("mov {}, fs:[0]", out(reg) value, options(nostack, readonly, preserves_flags)) };
    value
}

fn tls_write(value: u64) {
    unsafe { asm!("mov fs:[0], {}", in(reg) value, options(nostack, preserves_flags)) };
}

fn exit(status: u64) -> ! {
    unsafe { asm!("int 0x80", "ud2", in("rax") 1u64, in("rdi") status, options(noreturn)) }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    exit(250)
}
