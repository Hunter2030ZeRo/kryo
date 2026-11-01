# SMP execution and retirement

Date: 2026-11-01

## Scope

The SMP runtime supports one to 255 xAPIC logical processors. Larger inventories are rejected before HHDM protection changes. Directed xAPIC IDs are 0..254; 255 is broadcast. CPU sets span multiple 64-bit words. [ADR 0012](0012-growing-registries.md) replaces the earlier 64-slot, single-word capacity and adds growing execution registries. Limine supplies the processor inventory and AP entry stacks. CPU hotplug and x2APIC are not implemented. Bootloader memory remains reserved.

Three image-owned page-table pages establish a supervisor UC/NX APIC mapping before AP startup. APs first leave Limine's HHDM trampoline, load the shared boot root, initialize their local controller, and acknowledge a kernel-text parking loop that uses interruptible HLT. Only then may the BSP establish the HHDM NX policy. After memory and clock setup, the BSP releases parked APs to load the final protection state, calibrate already initialized local timers, flush their translations, and publish online state.

Each registered CPU owns its GDT, TSS, double-fault stack, default entry stack, user-entry bookkeeping, kernel FP save area, local timer configuration, and interrupt epochs. The immutable IDT is shared and loaded on every CPU. CPU indices are kernel identities and do not assume contiguous APIC IDs. A 256-entry reverse table makes trap-time identity lookup constant-time.

## Execution ownership

Core TaskTable records one current thread per CPU. Dispatch removes a Runnable thread from selection and records its sole Running owner. Return must name that CPU and thread generation. A Running thread cannot be stopped or reaped through the ordinary task operations.

The BSP coordinates each bounded execution batch. It retains exclusive access to process, task, capability, and IPC registries. A worker receives an owned mailbox payload containing a copied register context, moved FP/SIMD state, root address, entry-stack top, and deadline. It receives no references into those registries. The corresponding Running slots retain all root, page, and stack allocations.

Release publication transfers the payload to the worker; Acquire observes it. The worker saves user state and reloads the boot CR3 before returning the payload with the reverse ordering. The BSP may execute a job on CPU 0 while APs execute other jobs, including threads sharing one process root.

A batch-start barrier releases workers after all job wake IPIs have been delivered. Workers drain queued reschedule IPIs before user entry. A completed worker also publishes notification retirement after its BSP IPI has been delivered. This prevents a previous batch's notification from repeatedly preempting a new context before it executes.

The first completed job requests rescheduling on the other workers. Every dispatched job must acknowledge return before kernel policy resumes. A bounded watchdog halts on a missing acknowledgement while retaining execution resources. It never treats elapsed time as successful retirement.

Return processing has two phases. First restore every context and end every CPU ownership record. Then apply process-wide faults and exits before interpreting surviving native calls. A faulting process cannot continue a sibling's already trapped syscall.

## Mapping safety

A global execution-lease mask blocks VM changes while a worker can still use a user root. User mappings are non-global and PCID is disabled. Each worker's acknowledged boot-CR3 reload retires its user translations before the BSP can unmap or release backing storage.

Kernel and HHDM mapping changes also require a global TLB flush on every online CPU. A monotonic epoch is published before shootdown IPIs; each CPU flushes global and non-global translations before acknowledging it. The BSP waits for all targets. IRQ handlers do not acquire allocator, object, or scheduler locks.

An unmap whose shootdown fails retains its physical frame as pending retirement even after its PTE is removed. Retry must complete the flush before returning that frame. Mapping replacement and region detach reject pending retirement. HHDM split-table frames remain owned if their links have already been published.

## Waiting and idle

Native calls and wait transitions remain serialized on the BSP between batches. The existing Parking/completion handoff therefore finishes before another dispatch can publish the thread. Idle workers check mailboxes with IRQs disabled and use `sti; hlt; cli`; a published job sends a reschedule IPI. The BSP uses the local deadline timer while awaiting worker acknowledgements.

This design provides parallel user execution with a simple initial ownership boundary. It does not provide parallel kernel syscall processing, per-CPU run queues, work stealing, priority scheduling, CPU affinity, NUMA policy, or a performance guarantee. Those extensions require measured workloads and a separate synchronization design.

## Protocol reference

The [Limine MP protocol](https://github.com/limine-bootloader/limine-protocol/blob/trunk/PROTOCOL.md#mp-multiprocessor-feature) defines release/acquire AP startup, retained bootloader stack fields, and x86-64 AP entry state. Local dependency version 0.6.5 supplies `MpRequest` and `MpInfo::bootstrap`.

CPU capacity is separate from process and thread storage. Normal registries now grow with available memory; explicit bounded constructors retain the earlier small capacities for exhaustion fixtures. More CPUs can remain online and participate in TLB retirement even when fewer threads are runnable. CPU-local static storage scales with the configured maximum; Limine stacks additionally scale with actual processor count (512 KiB debug / 256 KiB release per processor). AP shutdown publishes STOP to all peers before one collective acknowledgement wait. The historical 64-slot storage measurements remain in the earlier device evidence and do not describe the 255-slot build.
