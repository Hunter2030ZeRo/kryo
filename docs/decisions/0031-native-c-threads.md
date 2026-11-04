# Native C thread calls

Date: 2026-11-04. Status: implemented native SDK and freestanding fixture.

## Existing kernel contract

The C SDK now exposes ThreadCreateNative (61), ThreadExit (33), ThreadJoin (34)
and ThreadJoinUntil (35). It adds no syscall or POSIX ABI. The memory descriptor
names runtime-owned stack/TLS ranges, FS base and an optional lower guard.
Creation fixes version one and flags zero. Compiler register constraints cover
all native operands and preserve SysV callee-saved registers R12 through R14.

The entry is an assembly bootstrap with a 16-byte-aligned initial stack and no
return address. It CALLs the C body. A normal C function cannot be passed as if
the kernel had already constructed its call frame. The runtime prepares its
TCB/TLS, supplies required language environment and finishes cleanup before exit.

Join returns status, full-width exit code and reason independently from RAX,
RDX and R8. A timeout is not completion. An expired deadline is checked before
terminal-result observation, so retrieving an old result uses untimed join or a
valid future deadline. Closing a control handle does not authorize early memory
reclamation. Stack/TLS backing belongs to the runtime after execution ends.

## Execution boundary

The mixed-language probe creates two successive C children using the same owned
private stack/TLS allocation. Its TLS word is a POD fixture accessed at FS:0;
it is not mlibc's TCB, a DTV or a compiler-TLS implementation. The assembly entry
captures the initial stack pointer for an alignment check in the C body.

The fixture checks arguments, live stack bounds, FS state, acquire/release park
predicates, timed join failure while the child remains alive, Busy retirement,
full-width high-bit exit codes, retained join results and stale handles. It
reuses stack backing only after native completion. Kernel validation observes
zero private/reserved charges at event 99/value 2, then publishes its CTHREAD
marker only after all 13 fixture threads, object references and owned frames
retire. The original private-word idle proof excludes this earlier C phase.

The debug executable needs 17 load pages. The bounded bootstrap admission cap
is now 32, with the existing segment/header/module bounds and per-process
mapping/table quotas retained. This does not establish a general hosted loader.

## Validation and remaining work

C11/C++23 checks compile the wrappers with live callee-saved register values at
O0/O2, in static and PIC/frame-pointer configurations. QEMU evidence separates
Haswell-v3 one-CPU tests from four-CPU normal startup. See the
[validation record](../validation/2026-11-04-c-threads/README.md).

This completes the C transport for existing native threads. mlibc startup,
allocator failure behavior, complete TCB/DTV lifetime, pthread cleanup and
service-backed descriptors remain separate integration work. It does not build
mlibc, libc++, Rust std, ICE or BetAFS.
