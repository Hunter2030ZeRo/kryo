# Native bootstrap stack and language ABI

Date: 2026-11-02. Status: initial stack direction accepted; freestanding probe implemented.

## Entry

General native startup uses the SysV AMD64 stack layout: argc, argv pointers,
NULL, environment pointers, NULL, auxiliary pairs and AT_NULL. Entry RSP is
16-byte aligned; a CALL then supplies the function-entry return address.
Copy argument and environment bytes into owned private storage before publishing
execution. Reject embedded NUL, invalid ranges and insufficient space before
writing the supplied stack buffer. The builder allocates nothing and does not
construct mappings or grant authority.

Cryolite startup authority remains separate: RDI identifies the existing
immutable version-1 StartupRecord capability. The runtime validates its header,
roles, actual object kinds and rights. Reusing this handle convention avoids
inventing an auxiliary-vector number. RDX is zero because no runtime finalizer
is supplied. Earlier boot programs keep their legacy argument registers and
zeroed initial stack; the new entry path is selected explicitly by image wiring.

The current probe supplies only truthful AT_PAGESZ and AT_ENTRY values. It does
not supply AT_RANDOM, mapped PHDR metadata, a general crt0, ELF TLS, constructors,
exceptions or dynamic linking. This finite freestanding gate is not a complete
hosted SysV process ABI or a libc/std sysroot.

## Compilers and state

The isolated user target selects x86-64-v3 and hard-float SysV calling conventions.
The kernel remains on its conservative soft-float target. Existing Rust boot
programs are not silently migrated. The probe cross-compiles core and compiler
builtins, including memory builtins, rather than adding fake libc implementations.

The builder checks the validated Rust commit and Clang version recorded in
`targets/cryolite-toolchain.json`. Packaging records compiler versions and
target/lock/artifact/image hashes, strips debug data and verifies that entry and
all load bytes are unchanged. This does not constitute upstream Rust target
support or official compiler/sysroot distribution.

The mixed ELF includes independently compiled C, C++ and Rust objects. It tests
mixed integer/double arguments, stack-passed floating arguments, C varargs,
mixed aggregate returns and bidirectional C-compatible calls. C++ uses an
extern-C boundary without the C++ standard library or exception runtime.

Live YMM0/YMM15 patterns cross yield and sleep. A separate single assembly block
loads vectors, emits a start marker, executes an AVX2 loop, emits an end marker
and stores the result. The kernel requires increased preemption ticks between
markers; final success follows status and vector validation. Only this task is
runnable, so no peer job completion can trigger a reschedule in that interval.
The experiment proves preservation under guest scheduling, not a latency or
performance bound.

## Validation integration

`make iso ABI_PROBE=1` and `boot_smoke.py --abi-probe` explicitly include the
probe module. Ordinary builds omit it and retain the earlier build dependencies.
The kernel skips the optional module before allocating execution resources.
No multi-million-instruction validation loop is added to ordinary startup.

Probe startup installs only Debug WRITE and StartupRecord READ before Runnable
publication. Exit validation checks terminal status, markers, mapping/handle/
wait/thread retirement and frame restoration after retained-table accounting.
The same probe path can run in normal and boot-test kernels.

Each smoke run now owns its kernel target directory. Cargo's per-build lock
alone cannot protect an ISO recipe while another recipe replaces a shared
profile artifact between kernel build and image copy. Isolated outputs prevent
normal and boot-test kernels from being mixed during parallel verification.

## Next mlibc gates

mlibc is the primary native port candidate. It still needs accessible ELF
headers/program headers or a reviewed equivalent, real TLS/TCB initialization,
private VM allocation, race-free waits and service-backed streams/files.
The inspected upstream static loader derives metadata from `__ehdr_start`;
dynamic startup also consumes auxiliary PHDR values. Recheck that behavior at
the chosen pinned mlibc revision instead of fabricating an auxv entry.

POSIX-facing types/constants, TLS/TCB layout, private memory and atomic-wait
semantics remain design-review items. Keep those native contracts independent
of a specific libc implementation and of ICE-Linux.

References: [x86-64 psABI](https://gitlab.com/x86-psABIs/x86-64-ABI),
[mlibc static/dynamic startup](https://github.com/managarm/mlibc/blob/master/options/rtld/generic/main.cpp),
[native runtime direction](0019-native-runtime-and-standard-libraries.md).
