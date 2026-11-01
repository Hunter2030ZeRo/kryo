# 0009: BSP user threads and independent process lifetime

Date: 2026-11-01

The BSP synchronization boundary below describes this implementation stage. [ADR 0010](0010-smp-execution.md) supersedes it for multi-CPU execution.

## Ownership

Keep one bounded process registry and one thread scheduler for their complete
runtime lifetime. Generational ProcessId and TaskId name different resources;
neither grants user authority. ProcessResources owns one address space, one
shared handle table, process control, and retained unpublished creation state.
Each ThreadResources owns its CPU context, FP/SIMD area, kernel entry stack,
managed user-stack/TLS slot, wait state, and optional thread-control reference.

Attach a new thread to its process while it is suspended. Bind control identity
before making it Runnable. Mark execution ended only after the CPU has returned
to the boot root and saved context completely. Retain membership until private
thread cleanup succeeds. A failed stack or mapping cleanup keeps the process
and thread generation occupied; it cannot free the shared address space early.

After the final member's private stack is retired, close process resources and
detach/reap the last member and process record. Old process and thread controls
retain terminal results independently of scheduler slots and VM resources.
Discarding an unreaped registry retains ownership rather than freeing live RAM.

The initial image runtime allows two processes, two threads per process, four
scheduled threads, and four waits. Core registries remain capacity-parameterized.
The 16-page user-mapping limit can reject another thread for a larger image;
this is an ordinary resource error.

## Creation and startup

ThreadCreate starts a thread in the caller's process. It requires a lower-half
entry address in an existing RX mapping. The kernel privately prepares a zeroed
four-page stack with a lower guard, a zeroed TLS page, a guarded kernel entry
stack, and initialized extended CPU state. Reserve result-handle capacity and
prepare control state before publication. Failed preparation explicitly retires
its resources, or retains its partial owners for cleanup retry.

The two user stack bases are 0x800000 and 0x820000; each has four pages and an
unmapped lower guard. TLS pages are 0x900000 and 0x920000. The whole managed
arena is excluded from ELF loading and native MemoryMap/MemoryUnmap. A thread's
slot is reused only after its old mappings and kernel stack have retired.
The native ELF's first thread owns its managed stack/TLS just like later threads.
Legacy scalar fixtures retain their earlier manually prepared stack layout.

At raw thread entry RDI is the argument, RSI is the TLS address, RDX bit 0 is the
AVX-state flag, and RSP is 16-byte aligned. FS initially points to the TLS page.
The entry must be a trampoline which calls an ordinary Rust/C function with
the required call-stack alignment. Returning without ThreadExit is a user fault.
This is manual FS storage; ELF TLS templates and a general std runtime remain
pending. Each thread's FP/SIMD and FS/GS state survives yields and preemption.

## Exit, stop, and observation

ThreadExit ends only its caller. ThreadJoin and ThreadJoinUntil observe the
retained result, including after private resources and the thread slot are
reclaimed. Self-join is denied. A timed-out or cancelled join leaves the target
running; closing its handle does not stop it. TaskStatus/TaskStop/CancelWait also
accept a thread-control capability, selecting that exact thread generation.

Existing Exit, fatal user exceptions, and process-control stop end the entire
process. The first process exit reason wins. Stop all members and detach their
waits before reaping resources. Publish process exit only when every member has
ended execution; publication still does not promise cleanup or DMA quiescence.
If all threads leave through ThreadExit, the final thread's exit status becomes
the process result. Waiting on one's own process is denied for every member.

Process-control cancellation must not arbitrarily select among several waiting
threads. The initial dispatcher rejects ambiguous requests; a thread-control
capability selects a specific wait. Concurrent close/use of a shared handle is
serialized on the BSP, and every lookup retains its existing strong reference.

## BSP synchronization and IPC limits

The scheduler runs one thread at a time with kernel IRQs disabled. A user return
ends CPU access before kernel operations mutate the shared process VM. Borrowing
the process space and the selected thread separately pins both during entry.
This remains the inactive-address-space contract of the current VM backend.

Legacy endpoint RECEIVE and notification WAIT ownership still names their
initial thread. Sharing a handle does not bypass that owner check. Owner-thread
termination closes those legacy objects even if sibling threads remain. General
process-owned receive queues, multi-receiver endpoints, and WAIT delegation are
separate work. Thread joins, sleep, and exit observation use the common bounded
wait table and first-completion/timeout rules.

## SMP follows this stage

This decision introduces OS threads on the BSP. It does not enable additional
CPUs. SMP needs per-CPU GDT/TSS/stacks, entry and FP-save state, run-queue ownership,
remote wakeup/IPIs, locking, and acknowledged cross-CPU TLB retirement. Disabling
local IRQs alone is insufficient. The existing BSP HAL rejects unsupported CPU
execution; AP startup must not bypass those checks.

Driver work remains independent of ICE's user-space ABI compatibility. Later
driver ports need native device authority and lifecycle interfaces rather than
an assumption that a Linux kernel driver can execute through ICE.
