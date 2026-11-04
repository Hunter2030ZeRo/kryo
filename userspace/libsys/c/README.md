# Native C call wrappers

[`cryolite/native.h`](../include/cryolite/native.h) exposes a small freestanding
C11/C++23 subset of Kryo's experimental native ABI version zero. It has no libc,
host-header or privileged-kernel dependency. It is not a POSIX header set or a
stable sysroot release.

Build native user code with the selected x86-64-v3 LP64 profile, MMX generation
disabled and the red zone disabled. The current C/C++ probe uses Clang with
`--target=x86_64-unknown-none -march=x86-64-v3 -mno-mmx -mno-red-zone`.
These flags do not apply to the kernel's separate soft-float compilation.

## Results and authority

Every returning wrapper yields `kryo_result`: signed 64-bit `status` followed by
unsigned 64-bit `value`. The call returns those fields independently in RAX and
RDX. Test `status == KRYO_STATUS_OK` before using the value. A successful handle
or value may have its high bit set; its sign does not indicate failure.
`KRYO_STATUS_*` constants are native negative statuses, not public errno values.
The libc adapter must translate errors according to the requested operation.

A `kryo_handle` is an opaque process-local capability, not a file descriptor or
PID. Zero is invalid. StartupRecord is a separately granted handle arriving in
RDI at native process entry; the SysV argc/argv/envp/auxv stack remains separate.
`kryo_startup_read` reads its indexed words. Consumers validate magic, version,
length, unique roles, kind-specific rights and handles before using grants.
The header exports the current startup bounds and DebugConsole kind for this
small interface; it does not declare every object kind or capability operation.

The inline assembly declares condition-code and compiler-memory barriers and
the result registers used by the native call boundary. Kernel entry preserves
the user FP/SIMD context, so callers may retain their normal SSE/AVX values.
There is no Linux syscall fallback or per-call lower-ISA dispatch.

## Memory, waits and diagnostics

Reserve takes nonzero page-multiple bytes and a power-of-two alignment of at
least 4096. Commit supplies actual zeroed RW/NX backing. Decommit retires backing
while preserving the reservation; release retires the whole reservation at its
exact base. These are native ownership operations, not complete mmap/munmap or
mprotect semantics. End all accesses before retirement, observe real thread
completion before reclaiming stacks/TLS, and retain cleanup ownership on error.

`kryo_set_tls` changes FS base without interpreting the runtime's TCB. The caller
must preserve valid TLS backing and compiler/runtime invariants. Clock and
sleep use the native monotonic nanosecond clock, not epoch or CPU time.

Park requires a four-byte-aligned word in private RW memory. It returns
`KRYO_PARK_WOKEN` or `KRYO_PARK_CHANGED` on success; a wake is not lock ownership.
Loop around an Acquire predicate and Release-store before waking. Wake buffers
no event. The timed form takes an absolute monotonic deadline. Shared-word waits
and POSIX pthread semantics are not provided by these wrappers.

DebugLog emits a numeric event. DebugWrite accepts at most 256 bytes through a
DebugConsole capability with WRITE authority and returns the snapshotted count.
The kernel snapshots the complete admitted range before emitting an escaped
diagnostic line. This is not stdio, a terminal, VFS access or a byte stream.
The caller keeps the source mapped and readable until the call returns.

```c
kryo_status report(kryo_handle debug) {
    static const char text[] = "native C diagnostic";
    return kryo_debug_write(debug, text, sizeof(text) - 1).status;
}
```

## Runtime-owned threads

`kryo_thread_create_native` uses the existing version-one native thread contract.
Its `kryo_thread_memory` gives the stack base/length, TLS base/length, FS base and
optional lower guard length. The caller prepares the runtime TCB/TLS before
creation. The kernel leases the supplied private mappings and creates execution
state; it does not allocate or free user backing.

The entry pointer names a raw assembly bootstrap, not an ordinary C function.
It starts with RDI=argument, RSI=TLS base, RDX=CPU feature flags, FS installed and
RSP at the 16-byte-aligned stack end **without a return address**. The bootstrap
must CALL the language entry and never return. Code, argument storage, stack,
TLS and shared values remain valid and synchronized through actual completion.
The wrapper fixes native version one and flags zero; its register constraints
preserve the C ABI, including the callee-saved registers used by this call.

`kryo_thread_exit` terminates the current thread without unwinding; complete
language cleanup and TLS destructors first. `kryo_thread_join` and the absolute
monotonic `kryo_thread_join_until` return `kryo_thread_result`: status from RAX,
the full 64-bit exit code from RDX and the separate reason from R8. Reasons are
Exited, Fault and Stopped. `kryo_thread_result_valid` accepts only successful
responses with a known reason. A nonzero status, including timeout, does not
establish completion and does not authorize backing reuse.

Observe successful native completion before reclaiming stack/TLS, then close the
thread handle separately. Closing a handle or observing a user flag cannot
replace join. These calls do not supply a TCB implementation, allocator, POSIX
attributes, environment inheritance, cleanup handlers or pthread semantics.

The header still omits IPC grant interfaces. Their many-register contract needs
its own reviewed shim before a general C runtime can use capability transfer.
