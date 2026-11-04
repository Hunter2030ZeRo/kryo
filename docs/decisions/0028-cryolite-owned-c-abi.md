# Cryolite-owned C ABI

Date: 2026-11-04. Status: foundation selected; detailed registries proposed.

The maintainer selected a Cryolite-owned ABI instead of importing mlibc's Linux
ABI headers. This proposal defines the foundation and the remaining release
gates. It does not publish a stable C ABI, install a sysroot or adopt mlibc.
The [candidate profile](../abi/c-v1.md) gives exact scalar and time layouts.
The [error](../abi/c-v1-errors.md), [memory/time](../abi/c-v1-memory-time.md),
[file](../abi/c-v1-files.md) and [signal](../abi/c-v1-signals.md) proposals now
give explicit numbers, layouts and unsupported-operation boundaries. Their
compile fixtures remain validation inputs rather than installed headers.

## Boundaries

Keep three contracts separate:

1. The C execution ABI uses the selected x86-64-v3/SysV AMD64 platform, LP64
   data model, ELF and runtime-owned static TLS.
2. Cryolite's public libc ABI defines POSIX-facing types, constants, structures
   and observable behavior. Cryolite owns their registry and version history.
3. Kryo calls and service protocols use their own statuses, capabilities,
   fixed-width records and explicit encodings. The current experimental native
   ABI remains version 0, unchanged by this proposal.

A C structure is not automatically an IPC record. Do not transmit compiler
padding, C enums, pointers, size_t or provider-private pthread objects as wire
data. Existing Rust Metadata, Entry, Grant, ProcessExit and ThreadMemory types
are not C overlays. StartupRecord stays an indexed, versioned word encoding.

Ownership means deliberate definitions and compatibility control. It does not
require changing values that a standard specifies, or assigning different
numbers solely to distinguish Cryolite. Linux headers and Linux syscall
semantics are not implicit fallbacks. ICE remains an independent translation
path.

## Foundation proposal

- Eight-bit bytes, little-endian LP64. Compiler-derived size_t and ptrdiff_t
  must match the selected compiler, including C++ type identity.
- Native user code targets v3 with MMX generation disabled. Retain 80-bit long
  double and the necessary x87 environment/state boundary; ordinary arithmetic
  uses SSE/AVX. Performance is a primary requirement under
  [ADR 0022](0022-platform-baseline-and-performance.md).
- Signed 64-bit file offsets and wall-time seconds. Public pid_t and clockid_t
  are signed 32-bit integers. A POSIX PID is a descriptive namespace entry,
  not a truncated native TaskId or an allocation capability.
- A timespec contains time_t tv_sec and long tv_nsec: 16 bytes, alignment 8,
  with fields at offsets 0 and 8 in this profile.
- Native negative statuses and libc's positive errno values are separate.
  Error translation depends on the operation and delivery state.
- Public VM, seek and clock constants get an owned registry. Their values do
  not pass through directly to kernel flags or service opcodes.
- Standard-mandated permission values remain standard values. Other layouts
  and constants require explicit entries; unspecified values are not inherited
  from a host header or another mlibc port.

No new C headers are installed while this profile remains a proposal. In
particular, the errno numeric registry, file/signal structures and provider
object ABI must be reviewed before claiming a complete libc ABI.

## Provider boundary

Evaluate mlibc at commit `07cfd943977737b8147c1289922c23479feef53d`.
This pin is an evaluation input, not a provider adoption decision.

The port needs more than its sixteen mandatory sysdep operations. The demo
port installs fifty ABI headers, and internal sysdep signatures include file,
signal and resource types unconditionally. Its loader creates struct stat
objects and reads device/inode fields. Deferring those layouts therefore also
defers an unmodified provider build; placeholder structures must not hide that
gate.

The provider owns its TCB, DTV, compiler errno storage and pthread object
representation. These still affect application ABI and must be versioned with
the eventual libc profile, but they are not kernel or service records. At the
evaluated pin, pthread_t is a pointer to provider thread data; it must not become
a native control handle through a cast.

The evaluated provider's mutex ownership fields use thirty-bit descriptive
thread IDs. Its missing-FutexTid fallback returns one. A threaded port must
supply unique provider IDs and real completion-aware lifetime management;
neither this provider constraint nor a PID representation narrows Kryo's
generational IDs or imposes a kernel-wide thread-count limit.

The provider publishes didExit before entering its ThreadExit sysdep. Runtime
stack/TCB reclamation must still observe Kryo's actual native completion. A
store to didExit alone does not authorize reuse.

## Implementation order

1. Review the foundation profile and compile its layout assertions with the
   actual target compiler. This checks representation, not bare-metal behavior.
2. Assign and review the complete errno and public flag registries. Define file,
   signal-set, resource and other ABI types needed by the chosen provider build.
   Keep their C views separate from BetAFS/service metadata and IPC records.
3. Create owned ABI headers and shared native C wrappers; compare C/C++/Rust
   encodings and reject unsupported versions, flags and conversions.
4. Pin provider dependencies and compiler inputs, then build the real static
   library. Run allocation, constructors, errno, TLS and negative cases on Kryo
   before claiming libc execution. A headers-only or host build is separate.
5. Add pthread completion/cleanup and service-backed descriptors. Rust std,
   C++ runtime, dynamic linking, terminal and KDE remain later deliverables.

The design review must cover full and partial munmap, protection changes,
entropy, wall time and unsupported operations. Current whole-reservation
release and RW private commit do not silently provide every mmap/munmap or
mprotect behavior. Capability diagnostics do not stand in for standard streams.

## References

- [mlibc ABI selection guide](https://docs.managarm.org/mlibc-book/porting/implementing_sysdeps_p1.html)
- [mlibc required sysdeps and startup](https://docs.managarm.org/mlibc-book/porting/implementing_sysdeps_p2.html)
- [Evaluated source](https://github.com/managarm/mlibc/tree/07cfd943977737b8147c1289922c23479feef53d)
- [POSIX type requirements, licensed republication](https://man7.org/linux/man-pages/man0/sys_types.h.0p.html)
- [POSIX errno requirements, licensed republication](https://man7.org/linux/man-pages/man0/errno.h.0p.html)
- [POSIX file mode requirements, licensed republication](https://man7.org/linux/man-pages/man0/sys_stat.h.0p.html)
