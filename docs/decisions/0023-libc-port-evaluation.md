# Native libc port evaluation

Date: 2026-11-02. Status: mlibc evaluated first; provider not finalized.

## Comparison

| Candidate | Verified upstream boundary | Cryolite work to evaluate |
| --- | --- | --- |
| mlibc | OS-specific sysdeps and ABI headers; configurable POSIX/Linux options; documented Mesa/Wayland users | Native sysdeps, startup/TLS and required POSIX services, pinned C++ build dependencies and upstream maintenance |
| musl | Built on the Linux syscall layer; existing Linux deployment ecosystem | Replace/adapt the OS assumptions while preserving thread, memory, signal and loader semantics; do not silently route native libc through ICE |
| relibc | Rust implementation with Linux and Redox backends; separate Redox runtime | Add a Cryolite backend and validate target/bootstrap dependencies, startup, pthread/TLS and compatibility |

mlibc's explicit new-OS boundary makes it a reasonable first port to evaluate.
This is an engineering assessment, not evidence that it is faster, smaller,
safer or more complete than either alternative. A documented application working
on another mlibc platform does not prove it works on Cryolite. Rust implementation
alone does not make relibc compatible with Kryo or supply Cryolite Rust std.

Do not rank these libraries by language or reputation. Compare patch scope,
upstream churn, actual target tests, startup/idle/RSS costs, TLS/thread behavior,
allocation and I/O costs, and the application's required extensions. Final
selection requires a review of those results with the maintainer.

## First mlibc port boundary

The upstream guide requires 16 sysdep operations even for a minimal static
program. The following groups describe the actual Cryolite gaps.

| Required operations | Existing Kryo mechanism | Remaining contract |
| --- | --- | --- |
| Exit, LibcLog, LibcPanic | Native process exit and capability-scoped diagnostic events | Native C wrappers, bootstrap text diagnostic sink and runtime teardown |
| ClockGet | Monotonic clock and absolute sleeps | Selected clock IDs and errors; wall time needs a real service |
| TcbSet | Validated per-thread FS-base setup | Actual TCB layout, ELF TLS template allocation and destructor lifetime |
| AnonAllocate, AnonFree, VmMap, VmUnmap | Owned zeroed memory objects and mappings | General private heap/VM contract, sizes, alignment, protection, errors and reclaim; MemoryCreate currently permits only 1..8 shared pages |
| FutexWait, FutexWake | Capability waits and coalescing notifications | Atomic compare-before-park or equivalent race-free address-wait protocol and lifetime rules |
| Open, Read, Write, Seek, Close | IPC and persistent block service | Runtime descriptors over VFS/stream capabilities, offsets/duplication, partial I/O and errors |

The initial stack must contain defined argv/envp/auxiliary information. Existing
manifest RDI startup is not that stack. Preserve capability startup through an
explicit versioned handoff rather than inventing POSIX file descriptors by
casting kernel handles. PT_PHDR and entropy auxiliary values need real backing.

Pthreads additionally need native thread startup/exit, stack preparation, TCB/TLS
initialization and join/destructor rules. Existing ThreadCreate/Join are useful
mechanisms, not an implementation of pthread or std::thread. A notification
alone does not provide futex compare-and-block semantics.

The current upstream build uses a freestanding C++ implementation and supporting
headers/libraries. This must be bootstrapped with a suitable target C/C++ compiler;
it does not require a completed native libc++ runtime first. Pin the mlibc
revision, dependencies and compiler, then validate the actual build. No provider
source or toolchain was installed by this evaluation.

## Decisions before publishing the ABI

- Select the native initial stack, TLS/TCB and capability handoff layouts.
- Decide whether to reuse selected POSIX-facing ABI constants/structures or own
  Cryolite copies. Upstream permits reuse without Linux syscalls but warns that
  its shared ABI headers can change. Freeze and version only the selected
  Cryolite contract; do not import every Linux ABI assumption by default.
- Define private VM and atomic wait semantics independently of one libc's
  implementation. Keep fd tables and normal allocators in user space.
- Review the chosen compiler/runtime combination and pinned source/dependencies
  before creating a reproducible target sysroot.

## Verification order

1. Actual v3 C/Rust/C++ calling-boundary tests and static ELF startup.
2. Honest static C execution with argument storage, exit status, constructors,
   zeroed allocation, per-thread TLS and negative resource cases.
3. Pthread/wait races, service-backed standard streams and VFS descriptors.
4. Rust std clients and the independent BetAFS native adapter, including its
   C codec dependencies. Then expand C++ runtime, dynamic linking and graphics.

Unsupported operations must return defined errors. Do not use successful stubs
or diagnostic event IDs as a working stdio/VFS implementation. Build-only and
host libc tests are separate from actual Kryo execution evidence. No candidate
libc has yet been built or executed on Kryo.

## Sources

- [mlibc README](https://github.com/managarm/mlibc/blob/master/README.md)
- [mlibc sysdeps and ABI selection](https://docs.managarm.org/mlibc-book/porting/implementing_sysdeps_p1.html)
- [mlibc required sysdeps and static startup](https://docs.managarm.org/mlibc-book/porting/implementing_sysdeps_p2.html)
- [mlibc target toolchain](https://docs.managarm.org/mlibc-book/porting/toolchain.html)
- [musl platform boundary](https://wiki.musl-libc.org/supported-platforms)
- [relibc README and platform backends](https://github.com/redox-os/relibc/blob/master/README.md)
