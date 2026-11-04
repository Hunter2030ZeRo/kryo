# Static mlibc bring-up

Date: 2026-11-04. Status: experimental ANSI/static implementation.

[ADR 0035](0035-provider-thread-lifetime.md) extends this initial milestone with
provider threads; the single-provider-thread limit below describes the original
bring-up boundary.

Cryolite now builds and executes pinned upstream mlibc rather than a substitute
set of C functions. The port keeps Kryo's native capability calls and the owned
C ABI proposals. POSIX, Linux, BSD and glibc build options remain disabled.
This profile establishes an initial C runtime; it is not a production libc
release or a completed hosted environment.

## Provider and allocation

The source manifest pins mlibc and all four freestanding dependencies by commit
and archive SHA-256. Builds use isolated target headers, the validated compiler,
x86-64-v3 and no MMX. The build applies the provider's supplied freestanding
header patches and a small local patch series. It checks generated-source
drift before cache reuse and installs into a fresh generated sysroot.

The maintainer selected Talc reuse through a thin C boundary. The provider's
ordinary allocator now calls unchanged Talc 5.1.1 in a separate `no_std` Rust
archive. A checked prefix retains each allocation's size, alignment and Talc
layout. Overflow and backing exhaustion return null; failed realloc preserves
the original object. A TLS-independent atomic/park mutex serializes the heap.
Contended waiting falls back to yield if wait credits are exhausted.

Heap arenas use direct private reserve/commit/release. They do not depend on
libc allocation or mapping callbacks. Failed commit rolls back its reservation;
failed backing retirement terminates the process rather than losing an owner.
The global heap remains alive until process retirement. Its first metadata
arena is retained. This is one shared heap, without per-CPU caches.

The static loader retains its upstream startup allocator. Failure during that
initialization remains a process-start failure, before a C program can receive
malloc errors. It is separate from the fallible allocator used by normal C
allocations.

## Startup and supported operations

An x86-64 crt preserves RDI's startup capability separately from the SysV
argument stack. The port validates the record and actual handle information.
Upstream `__dlapi_enter` reads the mapped ELF/PHDR/TLS template, creates its
initial TCB, installs FS, then runs constructors. Upstream exit handles atexit
callbacks and destructors before native process exit.

The port provides private anonymous RW mappings, private word waits, monotonic
time, relative sleep, yield and capability-scoped diagnostics. Mapping callbacks
retain a nonallocating registry and accept only exact bases and rounded lengths
on release. File operations return ENOSYS or EBADF; no descriptor table or
standard streams are installed. Snprintf formats into user memory; debug text
uses native diagnostics, not stdout.

The patch series also admits implementation-only locale/time/stdio declarations
in the ANSI build, omits unselected errno cases, checks signal numbers before
signed subtraction, and rejects unsupported thread creation before allocating
an unpublished TCB. Strlen and strcmp use upstream AVX2 implementations directly;
unused lower-ISA dispatch is absent from the linked probe. Linked code is checked
for MMX and the selected string paths.

The boot-native image policy now admits up to 4096 load pages, further bounded
by its mapping account. The earlier 32-page policy remains for legacy fixtures.
Mapping, table and memory failures retain their existing cleanup paths. This
raises a bootstrap admission budget; it does not provide a general executable
loader, dynamic linking or arbitrary process address layouts.

## Limits and evidence

Only the initial thread may enter the provider. Full pthread/C11 thread creation
needs unique lock-owner IDs, provider TCBs and completion-aware cleanup. The
separate allocator fixture creates two raw native threads with independent
dummy FS storage; they call only Talc and native wrappers. This tests allocator
concurrency without claiming provider thread support.

No entropy is fabricated. Stack-protector instrumentation is disabled and the
provider's fallback canary is deterministic. Real entropy and hardened startup
remain production requirements. Resource and scheduling type declarations only
close the static provider dependency graph; corresponding services and a stable
public ABI are not supplied. Rust std, C++ standard libraries, VFS, PTYs and
dynamic linking remain separate work.

The [validation record](../validation/2026-11-04-static-mlibc/README.md) identifies
the tested sources, target builds and QEMU cases. Tests cover arguments,
constructors, initialized/zero TLS, errno, alignment, zeroed allocation, real
quota exhaustion, preserved failed realloc, formatting, page-boundary AVX2
strings, sleep, unsupported threads/files, atexit/destructors, raw allocator
threads and final resource retirement. These are functional model/emulator
results, not hardware performance or power measurements.
