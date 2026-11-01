# ADR 0006: Native ELF loading and extended user state

Date: 2026-10-31

Status: Implemented and exercised by BSP boot-tests.

## Native image plan

Core parses static little-endian AMD64 ELF64 ET_EXEC into a bounded, borrowed page plan. It performs no allocation or unsafe file reads. Header and program-table sizes, file bounds, checked arithmetic, page alignment, lower-half addresses, flags, overlap, page budget, and file-backed executable entry are validated before mapping. Writable-executable pages, dynamic/interpreted images, PT_TLS, and unsupported program headers are rejected. The format rules follow the [System V gABI](https://gabi.xinuos.com/elf/08-pheader.html).

Image code rejects overlap with its user stack, lower guard, and TLS page before allocating mappings. Each plan page clears a full temporary page, copies its checked file subrange, and publishes it through the existing owned VM path. File padding and BSS remain zero. On mapping failure, the caller retains the address space for explicit cleanup; the trusted boot test retires it before reporting failure.

Limine supplies a module tagged `kryo-native-smoke`. Selection rejects missing, duplicate, empty, or greater-than-1-MiB module data. Module pages remain reserved while the borrowed plan is used. Native tests permit eight ELF pages plus four user-stack pages and one TLS page within the existing 16-mapping limit.

`userspace/native-smoke` builds separately from the kernel workspace. Its linker emits three RX/RO/RW load segments, four mapped pages, and actual BSS. Kernel and user build flags and linker scripts remain separate. The user entry receives identity, TLS address, and enabled AVX indication in RDI/RSI/RDX; RSP is 16-byte aligned before its assembly entry calls Rust.

This is native startup, not Linux or NT execution. ICE remains independent and has not been connected. Dynamic linking, TLS templates, argc/argv/envp/auxv, a general user target, libc/std, and process creation are still pending.

## FP and SIMD ownership

Each extended context owns a private, non-copyable state area aligned to 64 bytes. CPUID selects FXSAVE64 fallback or standard XSAVE/XRSTOR for x87/SSE and optional AVX. The 1,024-byte area bounds supported layouts; only XCR0 bits 0..2 are permitted. Unsupported pre-existing XCR0 components are rejected before mutation. AVX-512, AMX, supervisor xstate, compacted formats, and XSAVEOPT are not enabled.

New areas are cleared, with initial x87 control word and MXCSR set explicitly. XSAVE headers retain the hardware-produced bitmap; XRSTOR handles components marked initial. No user API can edit the save image. These rules follow [Intel's extended-state specification](https://cdrdv2-public.intel.com/851056/253665-087-sdm-vol-1.pdf).

Raw trap stubs first capture every GPR, then eagerly save user extended state before Rust. Kernel code runs with TS set and remains compiled without FP/SIMD use. User state restores immediately before IRETQ. The return path restores kernel FP state under the selected mask, then original XCR0, raw EFER, CR4, and CR0. Temporarily clearing EFER.FFXSR prevents AMD fast-FXSAVE behavior from skipping kernel XMM state. Raw EFER preservation includes bits not named by the pinned register crate.

The extended-state borrow lasts through execution and capture. It moves only between runs. CPU 0 retains its own kernel save area; this implementation does not provide SMP state ownership. Legacy scalar entry keeps its #NM policy and is tested again after extended execution.

## FS setup

SetTls validates and changes the caller's saved FS base, which the next resume installs. Kernel or noncanonical values are rejected without changing the old base. Zero is permitted. User FSGSBASE access is disabled, and kernel FS/GS state is restored on return.

The test uses a separately mapped private page and explicit FS addressing. It does not claim ELF PT_TLS handling or compiler thread-local runtime support.

## Verification

Host tests cover malformed/truncated ELF data, arithmetic, permissions, overlapping pages, entry rules, BSS, page initialization, xstate layout/initialization, CPU selection, and FS validation.

The same separately compiled program runs in two private roots. It checks initialized data, RO data, all BSS words, and distinct final results. Assembly probes retain x87 values/control words, MXCSR, XMM values, optional YMM upper state, and FS data through native yields and real timer preemption. Invalid TLS requests are tested before and after valid setup. Both tasks terminate through native Exit; their frames and stacks are reclaimed.

Default and `-cpu max` QEMU runs cover FXSAVE fallback and XSAVE/AVX respectively in debug and release. The max-CPU runner requires an enabled-and-executed AVX marker. These are functional emulator results, not physical hardware, performance, full application ABI, or service-startup evidence. See [status](../status.md) and [evidence](../validation/2026-10-31-native/evidence.json).
