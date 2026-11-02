# Design contract

Version 0.7, 2026-11-02.

This document defines the resource and execution rules for Kryo. It includes requirements for subsystems that are not implemented yet. See [status](status.md) for current support and the [roadmap](roadmap.md) for completion criteria. Experimental calls use version 0; stable syscall numbers, wire layouts, and published ABI versions remain undecided.

## Resource ownership

| Resource | Owner |
| --- | --- |
| Physical frames | Frame pool and the objects holding allocations |
| RAM memory objects | Core kernel objects |
| Address spaces and mappings | Core VM subsystem, using HAL page-table and TLB operations |
| MMIO and DMA mappings | Core resource objects under device authority |
| CPU state, TSS, and IST | The owning CPU's local state |
| Thread kernel stack | The thread, with temporary execution ownership held by its CPU |
| Boot information | Image code until transferred into core-owned metadata |

Physical, virtual, and DMA addresses are distinct. RAM memory objects and MMIO regions are distinct. A safe API must not turn an arbitrary integer into frame or mapping authority.

### Physical memory

Ranges are half-open. Check address arithmetic for overflow. The initial page size is 4 KiB.

Only Usable RAM enters the initial frame pool. Align usable ranges inward, align reservations outward, merge reservations, and subtract their union. Exclude bootloader-reclaimable, ACPI, executable/module, framebuffer, reserved, bad, and unknown regions until their reclamation requirements are met.

Initialize allocation ownership once. Recomputing a memory plan must not mark live allocations free.

Size frame metadata from the normalized Usable-frame count before reserving its storage. Reserve the whole page-rounded backing span, including padding, and normalize the allocation plan again before activating the pool. The pool must never allocate its own metadata pages. Bootstrap storage must not depend on a heap whose allocation path already requires the frame pool.

Initialize reserved bytes before creating typed mutable slices. Retain their exclusive HHDM-backed borrows for the pool lifetime. Validate the full plan and buffer capacity before changing pool state; initialization failure returns the supplied buffers unchanged. Claim bootstrap initialization before carving or zeroing so a second invocation cannot overwrite active metadata.

Allocation order is not physical-address order. A caller that requires a physical interval must request allocation within that interval. The interval restricts selection from existing pool authority; it does not add unmanaged frames or admit excluded metadata pages.

Mappings retain backing-frame references. Closing a handle does not release a frame still used by a mapping, request, or pin. Unmap includes removal of access and completion of required TLB invalidation on every relevant CPU. Reuse is forbidden while stale translations can still access the frame.

DMA references are tracked separately. Device access must end before a pinned frame can be reclaimed.

Initialize a frame before exposing it to another process or device. Keep frame-allocation metadata separate from shared memory-object lifetime; a heap allocation or `Arc` per frame is not required.

### Metadata allocation

Process, thread, handle, object, wait, and VM metadata may grow with available memory. A compile-time fixture capacity must not become a normal service limit. Explicit quotas and address-space budgets remain valid policy limits and must be reported as resource errors.

Bind processes to trusted resource accounts before initial execution. New threads
and ordinary child creation inherit the account; selecting another account requires
explicit ASSIGN and delegation authority. Restart preserves usage. Charge shared
RAM through final payload retirement, handles through open/reserved slots, waits
through consumption, and threads through successful execution-resource cleanup.
Per-process mapping and page-table bounds include aliases and the root table.
These bounds do not imply exact kernel-memory or CPU-time accounting.

Reserve fallible metadata before publishing a runnable owner, process membership, accepted capability grant, or wait registration. Keep the unpublished owner on failure. Prepare the initial process membership and result-handle capacity before making its first thread runnable. Completion delivery must have the side-record storage it needs before the wait can block.

Growth must preserve IDs, generations, pending reservations, and strong references. Do not compact or discard vacant generational slots while stale IDs may exist. Retire a slot before generation wrap. Metadata may move only while the caller excludes all references into it; the SMP mailbox protocol passes owned CPU state and retains no registry references.

An allocation token grants ownership of a frame or contiguous run. Heap headers retain these tokens; raw physical addresses cannot reconstruct them. Metadata deallocation must not release live mappings or forgotten hardware owners. VM metadata may be discarded after successful explicit close; registry slot history remains until registry retirement.

### Mapping lifetime

A safe buffer or pixel-access object must retain ownership or a valid borrow of its mapping. Another safe operation must not invalidate that mapping while the object can access it.

The current `Framebuffer32` constructor requires one writer and a retained boot mapping. General VM interfaces must encode the corresponding lifetime.

Memory reclamation separates accepting new access, revoking access, and completing revocation. Release storage only after mapping, reference, pin, TLB, and device cleanup has finished. Do not wait for hardware or another CPU in `Drop`; use explicit cleanup and retain resources while it is pending. Pending resources remain subject to accounting and quotas.

### Executable memory

Kernel mappings are supervisor-only. User executable memory must satisfy W^X across aliases of the same backing frame.

Prepare executable images privately. Remove write access and finish device writes before publishing executable mappings. The initial design keeps the resulting execution state immutable. JIT and later permission transitions need a separate contract.

## CPU and thread state

CPU-local state owns the CPU ID, current thread, GDT/TSS/IST, idle and emergency stacks, entry context, and preemption state.

A thread owns its address-space reference, kernel stack, saved context, TLS, allowed FP/SIMD state, wait registrations, and termination state.

GDT/TSS storage must remain at stable addresses while in use. CPUs may share an immutable IDT, but each CPU must load its own required registers. A global initialization guard must not skip an AP's register setup.

Each registered CPU owns stable GDT/TSS storage, entry and emergency stacks, user-entry bookkeeping, and kernel FP storage. Each CPU loads its own registers and updates TSS.RSP0 when entering a thread. The shared IDT remains immutable after construction.

SYSCALL does not switch to a kernel stack automatically. HAL entry code must obtain a trusted CPU-local stack before calling Rust code. TSS privilege-entry setup and SYSCALL entry setup are separate requirements.

CPU membership and execution leases use CPU sets that span all configured logical slots. Atomic word snapshots are not a transaction across the whole set. Startup, dispatch, shutdown, and shootdown handshakes must keep membership changes separate from the execution and mapping operations that rely on a snapshot. Missing acknowledgements retain resources regardless of CPU index.

### Execution and waiting

Thread states distinguish New, Runnable, Running, Parking, Blocked, Dying, and Dead.

- A thread may run on at most one CPU.
- Parking starts before context saving and execution handoff have completed.
- A wakeup during Parking records a pending wake. It must not allow another CPU to run the thread before the handoff. Cancel the sleep or publish Runnable after the handoff.
- Reclaim a dying thread's stack only after its CPU stops using it.
- Event, timeout, cancellation, and peer-termination races must produce one completion for a wait.

Preserve the registers and FP/SIMD state required by the user ABI. Initialize CPU features and new-thread state before running general Rust, Go, or C runtimes.

### Traps and return

HAL handles entry, context saving, and hardware-state decoding. Core selects policy from the cause and task state. Image code or a static interface connects the two without a HAL-to-core dependency.

| Condition | Policy |
| --- | --- |
| Permitted resumable event | Validate state and resume |
| Recoverable user page fault | Resolve through VM/pager and resume |
| Fatal user exception | Terminate the task and notify its supervisor |
| Fault in a registered user-copy operation | Return a syscall error through a bounded fixup path |
| Unexpected kernel fault or double fault | Emergency diagnostics and halt/recovery policy |

User-copy faults can occur at CPL 0. Recovery requires a registered copy/fixup region; CS privilege alone is insufficient.

Use a validated IRETQ return path first. Check user addresses, selectors, and flags. SYSRET optimization requires its own return-state validation.

## Objects, handles, and rights

Internal object IDs and process-local handles have different roles. A user handle is an opaque 64-bit value; zero is invalid. Slot and generation encoding are internal.

The object table owns identity and lifetime. Each process's handle table owns references and rights.

A lookup validates the slot, generation, type, and required rights, then acquires a stable reference under synchronization. Keep the lock until that reference is secured. Retire slots before generation wrap can make old handles valid again.

Close removes one handle. It does not cancel all accepted operations or revoke every mapping. Duplication requires authority and can only preserve or reduce rights. Transfer reserves receiver capacity first: failure before acceptance leaves the sender's resource intact; acceptance commits ownership to the message or receiver.

READ, WRITE, MAP, SEND, RECEIVE, WAIT, SIGNAL, DUP, TRANSFER, and CONTROL are separate rights. Define valid combinations per object type. Ordinary memory MAP rights do not grant arbitrary RAM or MMIO access.

Object-wide revocation requires CONTROL. The initial design covers close and object-wide revocation; selective descendant revocation through a capability derivation tree is deferred.

Revocation blocks new work. Already accepted operations, mappings, and pins follow resource-specific cleanup. Server termination does not destroy shared memory objects still referenced elsewhere.

## IPC and shared memory

### Control messages

Start with synchronous endpoint send/receive/call/reply. Experimental version 0 implements 32 inline bytes and up to two capabilities through copy delegation with reduced rights. Source handles remain valid; linear move remains pending. Confirm limits and discovery before publishing a stable ABI; current experimental registers are documented in [interfaces](interfaces.md).

Store small pending messages in waiting-thread state and transfer them at rendezvous. Avoid unbounded mailboxes and a heap allocation per RPC. Waits and receiver capacity are subject to task/process quotas.

The current receiver owner is one thread. SEND rights may be delegated. Process-wide concurrent receiving and consumer migration are deferred.

Endpoint owner termination or explicit closure marks the endpoint Closed and resolves send, receive, and reply waits. Remaining capabilities do not reopen it.

Native channel creation requires an explicitly granted factory with CREATE. Duplicates and delegated factory capabilities share their account. Charge each channel until its final payload reference retires, including references retained after closure or creator death. Remove closed registry roots after accepted grants and completions settle; close a channel when no external authority retains it. Device roots remain until explicit hardware retirement. The current consumer owner is the exact creating thread; RECEIVE and WAIT migration remain unsupported.

| Request state | Failure meaning |
| --- | --- |
| Before receiver acceptance | Not delivered; timeout, cancellation, or closure preserves original transfer resources |
| After acceptance | Work may have executed; timeout or server failure does not imply rollback |
| Reply committed | The request cannot complete again |

A call's reply token is single-use and tied to the correct wait and generation. Reject late replies after caller cancellation or termination and clean up retained resources.

Distinguish invalid handles, wrong types, permission failures, invalid ranges, memory/quota exhaustion, WouldBlock, timeout, cancellation, peer closure, and uncertain outcomes after delivery. Numeric error codes remain undecided.

### Notifications

The initial notification is a coalescing pending state for one consumer, not a message or completion counter.

1. The producer publishes queue data, then signals.
2. The consumer drains the queue, then waits.
3. Under one object lock, wait either consumes pending state or registers the waiter.
4. Signal records pending state or completes one registered wait.

Do not expose a separate unconditional clear operation. A signal before waiter registration remains pending; a signal after registration wakes the waiter. A redundant wake is acceptable; a lost wake is not.

Native waits initially cover one object. Do not constrain the internal model to Linux FD/poll. Atomic NT multi-object waits require a separate extension.

### Shared buffers

Bulk requests name a registered memory object, offset, length, and permitted access. Raw pointers are not process-independent wire values.

Initial request and completion queues are SPSC and bounded. Full queues return WouldBlock or enter a defined wait. Publish data with Release and observe it with Acquire; define slot-return ordering as well. Volatile access does not provide this synchronization.

Treat shared indices, lengths, and offsets as untrusted. Check rights, overflow, and bounds, then snapshot the metadata used for access. Do not re-read mutable fields as authority after validation. Shared access must not create incompatible Rust references.

A timed-out caller may still have a server or device using its buffer. Ending a wait and ending a data loan are separate operations. Retain the physical memory until all uses end.

## Services and devices

A user-space registry/supervisor manages stable service IDs. Distinguish service identity, process identity, endpoints, and instance generations.

Service IDs and message fields never authenticate authority. Bind registration to
a private instance admission capability and discovery policy to a private client
gateway, or provide an equivalent authenticated mechanism. Give lookup clients no
service-control operation through that gateway. Verify received capability kinds
and rights before publishing a service or forwarding a grant.

Validate startup roles against required roles, object kinds and maximum rights.
Install all startup authority and its immutable receipt before publishing child
execution. Revalidate descriptive source handles at spawn; stale handles invalidate
the entire attempt. Allocate generations before start attempts and reject wrap.
Keep service resource accounts across replacement until retained owners retire.

Requests distinguish the client instance, request ID, and target generation. Define identifier reuse and reject stale completions. Generations alone do not provide exactly-once execution or state restoration.

Orderly replacement stops new requests, resolves in-flight work, transfers required state, prepares a new instance, switches the registry, and reconnects clients. Driver replacement also requires device quiescence and authority revocation.

| Component | Failure responsibility |
| --- | --- |
| Kernel | Stop task execution, resolve handles/waits, restrict new access, retain live resource references |
| Supervisor/registry | Record failures, select restart policy, advance generations, reconnect clients |
| Device manager and authorized control path | Stop/reset devices and verify DMA isolation |
| Service | Recover available state and report successful, failed, or uncertain operations |

### DMA reclamation

Ring 3 may use its language's standard library through the native Cryolite
runtime. Kernel and HAL remain no_std; the filesystem boundary does not require
BetAFS to abandon std. Keep service records independent of Rust/C/C++ layouts.

Until isolated DMA domains exist, untrusted drivers may submit only validated
offset-based descriptions. Hardware queues and physical links stay private.
Snapshot accepted plans and input before publication; retain completion targets
until the device returns the chain or reset proves old access has ended.

PCI inventory is descriptive. Reading an assigned BAR gives neither its size nor
authority to map it, and discovery does not establish device quiescence. Keep
configuration access serialized and leave device state unchanged until an owned
initialization path takes responsibility for it.

Constrain both allocation authority and the complete device-visible address
range. Initialize backing before publication. Constructor cleanup failures must
return retained owners just as retirement failures do. A bounded allocation does
not provide driver isolation without a validated submission path or IOMMU domain.

Track Registered, InFlight, Quiescing, Quiescent, Unmapped, Unpinned, and Reclaimable states. Confirm transitions against device and IOMMU behavior.

Process termination and IRQ masking do not prove DMA has stopped. Block new device access, stop or reset the device as required, and drain in-flight transactions. Complete IOMMU/IOTLB work before releasing mapping and pin resources.

If quiescence cannot be established, offline the device and quarantine its pages. Do not reuse memory that a device can still access. Untrusted DMA-driver isolation requires both IOMMU support and this lifetime management.

Report uncertain writes as uncertain. Automatic retry requires a service contract for idempotence, deduplication, or durable request tracking.

## Synchronization and failure

Acquire multiple process handle tables in ProcessId order and multiple object locks in ObjectId order. Use handle-table-to-object lock order consistently.

Commit a wait completion and retain a stable thread reference before releasing the object lock and requesting a wakeup. Do not block or switch context while holding capability/object locks. SMP work must verify Parking handoff and multi-queue transitions.

Locks shared with IRQ handlers must prevent local IRQ reentry. Disabling local IRQs does not exclude another CPU. NMI/emergency paths must not depend on normal allocator, scheduler, or object locks.

Allocation failure, quota exhaustion, and peer termination are ordinary errors. User input must not panic the kernel. Roll back unpublished objects, mappings, and capabilities on failure; retain resources that require deferred hardware cleanup.

## Verification

Use [roadmap completion criteria](roadmap.md) for each subsystem. Test invalid input, overlap, overflow, exhaustion, stale rights, request races, and peer failure alongside successful behavior.

Host models verify structures and state transitions. QEMU verifies guest execution and hardware-facing paths. Physical hardware is required for performance, power, and device recovery measurements.
