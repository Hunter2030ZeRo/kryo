# C ABI registries and native code generation

Date: 2026-11-04. Contract: [C-v1](../../abi/c-v1.md) and
[platform requirements](../../decisions/0022-platform-baseline-and-performance.md).

## Implemented code-generation policy

The native Rust target disables MMX with `features=-mmx`. Its C/C++ peer build
uses `-march=x86-64-v3 -mno-mmx`. The kernel's soft-float target and hardware
admission remain unchanged. Cumulative v3 checks still require FPU/MMX hardware;
they do not select legacy library implementations. The retained x87 fixtures
and 80-bit long-double ABI remain distinct from ordinary SSE/AVX arithmetic.

Four QEMU TCG configurations passed with 512 MiB guest RAM:

| Profile | CPU / processors | Mode | Modules |
| --- | --- | --- | --- |
| debug | Haswell-noTSX / 1 | boot-tests | ABI probe |
| release | Haswell-noTSX / 1 | boot-tests | ABI probe |
| debug | max / 4 | normal | ABI probe, writable block fixture |
| release | max / 4 | normal | ABI probe, read-only block fixture |

Each run requires the existing mixed-language calls, compiler TLS, runtime-owned
threads, private VM, atomic park/wake, live YMM preemption and resource-retirement
markers. The normal SMP runs also retain the block-service checks. Linked
debug/release probe disassembly contains no MMX register or EMMS/FEMMS use.
The packaged images remain within the unchanged sixteen-LOAD-page limit.
Per-run JSON identifies the kernel, packaged ELF, build flags and raw logs.
The runner stops only its emulator; this does not establish guest shutdown.

## Candidate ABI checks

The four owned registries propose 79 distinct positive errno values and two
aliases, explicit VM/clock selectors, 128-byte stat and compact signal layouts.
The 8-byte signal set represents 64 positions. File allocation accounting uses
512-byte public units with checked conversion from actual BetAFS allocations;
logical file size is not an allocation count. Signal context descriptors keep
variable-length XSAVE data separate from the kernel's internal 1024-byte area.

`files.h`, `signals.h` and the layout sources are compile fixtures. They do not
install public headers, select a signal-return mechanism or implement a libc.
`check.py` parses the numeric/alias tables and compares file/signal definitions
with their fixtures. It checks 81 error names, 25 memory/time, 96 file and 46
table-defined signal constants. Signal-specific code groups and prose-only
constants remain reviewed definitions rather than this parser's inventory.

Eleven compiler checks passed using the pinned Clang 23.1.4 and Rust revision:

- Target C11/C++23 scalar and aggregate layouts, with warnings denied.
- C constant/preprocessor consistency and independent Rust host repr(C) layouts.
- v3 code generation: VADDSD, VPADDD, explicit fused multiply-add, LZCNT and BZHI;
  the 80-bit operation retains FLD/FADDP, with no MMX in the fixture.
- Expected compile rejection of MMX-enabled and v2 profiles and of 64/128-bit
  long-double substitutions. Rejection logs are retained.

These commands compile but do not execute the new candidate structures. Rust
layout checks use the x86_64-unknown-linux-gnu host target; they do not prove
Cryolite Rust std support. The code-generation fixture demonstrates availability
under v3, not speed, hardware behavior or a new production SIMD implementation.
No broad fast-math option or above-v3 requirement was enabled.

Reproduce the compile checks outside the source tree:

```sh
rtk proxy python3 docs/validation/2026-11-04-c-abi-registry/check.py --output /tmp/kryo-c-abi-check
```

[compile-evidence.json](compile-evidence.json) records commands, expected errors,
compiler versions, parsed constants and source hashes. [evidence.json](evidence.json)
indexes the emulator results, and [source-manifest.json](source-manifest.json)
identifies source/configuration inputs. Prior kernel host-model evidence is not
recounted as a fresh test run for this code-generation change.

## Remaining provider gates

No libc has been built or executed by this milestone. Resource, directory-entry,
terminal/network types, provider pthread representation and complete owned
headers still need review. The pinned provider additionally needs signal-number
prevalidation before signed subtraction, selected clock visibility and actual
thread completion before TCB/stack reuse. Layout compatibility does not remove
those requirements. POSIX signal delivery, general range unmap/protection,
epoch/CPU clocks, VFS streams and descriptors remain unimplemented integration
work. Unsupported behavior must fail honestly and preserve resource owners.
