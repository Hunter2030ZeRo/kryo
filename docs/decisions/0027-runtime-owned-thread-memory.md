# Runtime-owned thread memory

Date: 2026-11-03. Status: accepted; native mechanism implemented.

## Ownership

The user runtime prepares stacks, guard policy and TCB/TLS storage. Kryo creates
the guarded kernel entry stack and private CPU state, validates the supplied
user mappings and retains an execution lease before publishing a runnable
thread. The lease protects mapping lifetime; it does not transfer allocation
ownership or interpret libc-private TCB fields.

Legacy ThreadCreate (32) retains its managed stack and scratch-TLS convention.
ThreadCreateNative (61) uses runtime-provided memory without reserving a legacy
user stack/TLS slot. Thread/account limits, metadata allocation and the kernel
entry-stack address budget still apply.

## Register contract

The outer native ABI remains experimental version 0. Call 61 accepts this
version-1 scalar record directly in saved registers; no user descriptor pointer
is dereferenced.

| Register | Input |
| --- | --- |
| RDI | Version, 1 |
| RSI | Entry address in an RX mapping |
| RDX | Argument word |
| R8 / R9 | Stack base / byte length |
| R10 / R11 | TLS backing base / byte length |
| R12 | Initial FS base inside the TLS span |
| R13 | Lower guard byte length, zero or a multiple of 4096 |
| R14 | Flags, zero |

Return RAX is status and RDX is the thread-control handle. Unknown versions and
flags fail before creation. The handle retains the existing
INSPECT|CONTROL|WAIT|DUP|TRANSFER rights and generational terminal observation.

The child starts with RDI=argument, RSI=TLS base, RDX bit0=AVX enabled and
FS=the supplied base. RSP is the stack end, aligned to 16, without a return
address. An assembly trampoline must CALL the language entry and must not
return. FS is installed before the first child instruction. SetTls on this
thread accepts only addresses inside its leased TLS span; legacy behavior is
unchanged.

## Admission and protection

Stack base and end must be 16-byte aligned, with at least 16 bytes. This is a
low-level entry requirement, not POSIX's PTHREAD_STACK_MIN. The runtime selects
an appropriate language stack size. TLS may be a byte-granular packed block;
neither its base nor its length must occupy a whole page. FS must fall inside
that block. Both spans must be nonempty, checked user ranges backed throughout
by published private RW pages. Shared, RO, RX, missing and unpublished backing
is rejected. Legacy managed stack/TLS arenas are not accepted as native input.

Stack, TLS and guard spans cannot overlap each other or an existing execution
lease. Distinct packed TLS blocks may share a private page; page retirement
remains blocked while any covered block is leased. A nonzero guard requires a
page-aligned stack base and an entirely unmapped range immediately below it.
The guard remains protected against later map or commit, not merely checked at
creation. Large unmapped guards use interval metadata without allocating data
frames; reservation search must skip their full extent.

Core map, unmap, commit, decommit, release and address-space close reject covered
leased ranges with InUse. Native operations that reach this admission check
translate the conflict to Busy (-11), before invalidation, retiring flags or
quota changes. Legacy MemoryMap/MemoryUnmap first reject reserved private-VM
arenas with InvalidArgument, regardless of a lease. A leased page outside those
excluded arenas, such as static ELF TLS, reaches the Busy check. Other invalid
input keeps its existing error precedence.

Busy does not quarantine the process or cancel unrelated waits. Lease metadata
grows fallibly per thread, without a new owner or allocation per user page.
Dropping a lease token conservatively leaves its protection in place.

## Publication and completion

Reserve thread membership, result-handle capacity, account credit and lease
metadata before runnable publication. Failed creation publishes no thread.
Because that child never executed, rollback releases the user-memory lease
even if kernel entry-stack cleanup remains pending. Pending kernel owners and
their thread charge stay retained until cleanup succeeds. Runtime mappings are
not removed during rollback.

After all user CPU execution leases end, a Dying thread releases its memory
lease before its terminal result is published. Successful native join therefore
allows user stack/TLS reuse without waiting for later kernel-stack reaping.
The user mappings remain present after both exit and reaping. Closing a control
handle or observing a user didExit flag is not completion evidence.
The lease release does not refund thread credit: that charge remains with
kernel execution resources until their cleanup succeeds.

A private word waiter on runtime-owned storage need not be cancelled merely
because its thread owner exits: the backing is still mapped. Ordinary unmap or
decommit/release cancels those waits before retirement. Reusing a TCB or atomic
object in place inside an unchanged mapping requires the runtime to drain its
waiters; mapping IDs identify remapping, not language-object lifetime.

## POSIX and language runtimes

The raw kernel starts fresh FP/SIMD data state. A pthread runtime can snapshot
the creator's floating-point environment and install it in the startup
trampoline before calling the application. It also owns pthread attributes,
joinable/detached policy, cleanup handlers, TSD/TLS destructors and the language
return path. Signal-mask inheritance and the final-thread process-exit policy
need their own POSIX-facing implementation. TaskStop and CancelWait do not
implement pthread_cancel.

The ABI probe's two compiler-TLS children use this API with parent-prepared
TCBs, runtime stacks and protected guards. They test Busy before release,
actual join before reuse and rounding-control inheritance across yields and
sleep. The fixture covers x87/MXCSR rounding, not a complete POSIX floating-point
environment. Three compiler-TLS participants and eight legacy word-test
children retain eleven total thread retirements. Private VM usage returns to
zero before its separate allocation checks.

No libc/std port, complete pthread implementation, allocator efficiency or
hardware performance result follows from this mechanism. Validation belongs in
[status](../status.md).
