# Native C calls and bounded diagnostics

Date: 2026-11-04. Status: implemented bootstrap interfaces; libc integration pending.

## Native C boundary

The freestanding [C SDK](../../userspace/libsys/c/README.md) supplies a small
C11/C++23 subset of experimental native ABI zero. Calls use int 0x80 with
independent signed status in RAX and full-width value in RDX. A high-bit result
is not an error. The wrappers name the admitted operations and preserve compiler
memory ordering; the selected v3/MMX-disabled user build remains separate from
the kernel's soft-float target.

The SDK contains no Linux syscall fallback, POSIX errno translation or hidden
allocator. Ordinary scalar wrappers do not stand in for many-register calls.
[Native thread shims](0031-native-c-threads.md) now cover creation and completion;
IPC grants still require their own reviewed C transport.

## Diagnostic mechanism

Libc bring-up needs actual failure text before VFS and standard streams exist.
DebugWrite, call 62, uses explicit DebugConsole WRITE authority. It admits at
most 256 bytes, snapshots the whole owned published user range while CPU leases
are inactive, then emits one escaped record. Empty calls validate authority but
do not read memory or emit a line. Failure emits no partial data.

The fixed stack buffer belongs to a cold noninlined helper. Normal IPC, VM and
scheduling calls do not contain that buffer in their dispatch body. This is a
code-structure boundary, not a measured latency or stack-size result.
There is no allocation, stream state or user formatting interpreter. Control
bytes and backslashes are escaped so payloads cannot create physical kernel
log lines. Validators also remove USERDIAG records before matching trusted
kernel markers; escaping alone would not protect substring-based checks.

This diagnostic capability is not a descriptor, terminal or standard-output
implementation. The native runtime retains its service and capability model.

## Experimental public definitions

[C profile source headers](../../userspace/libc/abi/README.md) now implement the
reviewed candidate scalar, error, memory/time, file and signal views under an
explicit cryolite include path. They are not installed system headers, complete
mlibc abi-bits, function implementations or a stable ABI release. A pointer-only
incomplete pthread_attr_t declaration fixes no provider object representation.

The selected libc provider must still complete the resource/network/terminal
type dependency closure. Do not include host definitions to fill those gaps.
Native status translation remains operation-aware, with buffer and mapping
ownership preserved through actual retirement.

## Execution evidence

The optional mixed-language ELF now calls the native C wrappers from Ring 3.
Its owned startup argument has a high bit set, so an actual successful native
read checks status/value separation. A reduced-rights debug handle tests WRITE
denial. C exercises zeroed VM backing, decommit/recommit, full release, monotonic
clock, yield, sleep, private word mismatch/deadline/wake and malformed calls.

Diagnostic cases cover RO image bytes, a maximum-length two-page RW snapshot,
empty and oversized requests, invalid/supervisor addresses, an unreadable
second page and a released range. The payload contains attempted fatal and
success-marker text. A Rust wrapper emits a separate record. A kernel marker
follows actual task/object/frame retirement, independently of user event 98.
These checks remain distinct from installed libc, malloc, stdio and Rust std.

## Provider evaluation gates

The pinned mlibc source has additional concrete integration requirements:

- AnonAllocate failure is asserted by the default allocator at this revision.
  Returning ENOMEM from sysdeps alone does not make malloc exhaustion recoverable.
  Preserve ordinary OOM errors when designing the actual allocator port.
- Generic signal-set helpers need validation before subtracting one from a
  signed signal number. INT_MIN is an invalid-input regression case.
- The public time header exports additional clock names. Select their visibility
  and error behavior explicitly rather than inheriting a Linux-oriented view.
- Private lock IDs and actual thread completion must be integrated before
  publishing pthread support or reclaiming TCB/stack storage.
- Handwritten strlen/strcmp IFUNC paths include older-ISA variants. The native
  v3 policy requires reviewing those paths; a compiler flag does not remove
  handwritten dispatch.
- Pinned freestanding dependencies, x86 startup, real entropy policy and audited
  target compiler builtins remain required. The installed Clang's advertised
  bare-metal builtins archive is absent on this development host.

These are evaluation findings, not implemented provider patches or evidence
that the library has been built. mlibc remains the primary candidate. Its
allocator replacement and final adoption remain design decisions to review
with the maintainer.
