# ADR 0003: BSP preemption and task authority

Date: 2026-10-31

Status: Implemented and exercised by boot-tests; BSP scalar execution only.

## Context

Private address spaces protect memory but do not prevent an infinite user loop from owning the CPU. Independent services also need explicit task lifetime and authority. This stage adds those mechanisms before endpoint IPC and service startup.

## Execution ownership

HAL stores all 15 general registers, RIP, RSP, RFLAGS, and FS/GS bases in `UserContext`. Trap entry saves the complete register set before calling Rust. It clears DF and AC for kernel execution. IRETQ return validates user addresses and flags; user state cannot grant I/O privilege or disable preemption. Invalid saved state terminates that task through the image policy.

`run_context` temporarily owns the CPU, user root, and task entry stack. It updates TSS.RSP0, enters the user root, and returns a saved outcome for timer, native call, or fault. Before returning to its caller, it stops the timer and restores the boot root, stack, TSS.RSP0, and kernel register/control state.

Core's `TaskTable` holds resources and selects Runnable tasks in round-robin order. It permits one Running task. Image wiring holds an exclusive resource borrow across `run_context`, then calls `returned` only after the HAL has stopped execution and saved the context. This ordering prevents publishing Runnable or Dying while the CPU still uses task storage.

Timer/yield returns make a task Runnable. Native calls first make the caller Blocked; immediate completion wakes it unless the call stopped it. Fault/exit makes a task Dying. `reap` requires Dying and successful explicit cleanup before removing resources or advancing the slot generation. Cleanup failure retains resources for retry. Dropping a table retains unreaped resources rather than freeing possibly live storage.

This synchronous BSP handoff does not implement the Parking/wakeup protocol required for SMP or asynchronous waits. A task currently contains one execution context and one handle table; a general process/thread split remains future work.

## Kernel entry stacks

Each test task owns four RAM pages mapped supervisor RW/NX, with an unmapped lower guard page. A dedicated 2 MiB kernel region is attached after HHDM initialization and before task roots inherit the kernel half. Its page tables retain allocation tokens until all inheriting roots have been closed.

Stack cleanup runs after the HAL restores the boot stack and TSS.RSP0. It removes and invalidates each mapping before releasing its frame. The region also contains a separate supervisor RW/NX uncached local-APIC mapping; device memory is not entered into the RAM pool.

## Timer

The initial controller is BSP xAPIC. x2APIC and AP use are rejected. Initialization masks other supported local sources and the legacy PIC, sets the local priority, and installs timer vector 0xe0 and spurious vector 0xff. The controller mapping remains owned until shutdown.

User slices use a periodic timer with a count of 1,000,000. This is an uncalibrated hardware count, not a duration guarantee. Kernel code normally keeps maskable IRQs disabled. Timer entry acknowledges the local APIC and returns the saved user context to the scheduler.

Stopping the timer does not prove its vector is no longer queued. Shutdown masks the source and drains pending delivery through a bounded CPL0 IRQ window before clearing the MMIO reference. The kernel timer stub only acknowledges and returns; it cannot schedule. No allocator or object lock may be held across this window. A busy controller returns an error and retains its saved state and mapping for retry. Final cleanup restores saved controller/PIC configuration with IRQs disabled.

## Objects and authority

Core supplies a fixed-capacity object arena and process-local handle tables. `ObjectRef` retains a strong reference. Lookup validates handle generation, object kind, and rights before retaining the reference. Scheduler operations occur after releasing the object lock. Payload destruction occurs outside that lock.

Object IDs describe identity; they grant no authority. Handles are opaque 64-bit values, local to their owning table. Zero is invalid. Closing a handle releases one reference. Duplication requires DUP and cannot increase rights. Object, handle, and task slots retire before generation wrap.

TaskId and ObjectId are registry-local. Keep the task registry alive while any task-control references name it; IDs must not be routed to a replacement registry. Object references borrow their arena directly. The guest fixtures drop all controls before replacing their local task table.

The image exposes task-control objects with INSPECT and CONTROL rights through the experimental [version-0 calls](../interfaces.md). Task control contains a generational TaskId, not a raw resource pointer. A remaining control reference reports Exited after the task is reaped. Memory and Endpoint kind tags reserve no implemented operations or service authority.

## Verification and limits

Host tests cover scheduler ownership, cleanup retry, exhaustion, generation retirement, reference lifetime, stale handles, and rights attenuation. QEMU debug and release both demonstrate two noncooperative tasks receiving eight total timer slices, preserved caller/callee registers, capability denials, stale-handle rejection, invalid-stack task isolation, queued timer draining, and complete per-task reclamation. See [status](../status.md) and the [recorded evidence](../validation/2026-10-31-tasks/evidence.json).

Normal boots do not start these test tasks. Capacities in the guest fixture are two tasks, eight objects, and eight handles per task. There is no heap allocation per call.

This stage does not provide FP/SIMD context, SMP, a calibrated clock, asynchronous waits, IPC, shared memory, capability transfer, quotas, user-copy recovery, a stable ABI, or an executable user runtime. FP/SIMD remains trapped. ICE, BetAFS, drivers, and service startup require later integration.
