# ADR 0005: Shared backing and capability delegation

Date: 2026-10-31

Status: Implemented for bounded BSP execution and exercised by boot-tests.

## Shared memory ownership

`SharedMemory` owns a bounded array of allocation tokens. Creation zeroes each frame once and establishes writable, non-executable kernel aliases before publication. Invalid sizes and allocation/initialization failures are errors. Partial construction releases unpublished frames; the backend failure contract requires release-safe aliases and no outstanding access.

Memory objects remain immutable in shape after creation. A mapping retains a strong `SharedPage` lease to the whole object plus a validated page index. Image leases hold an `ObjectRef`; core VM stores that lease with its mapping. A physical address alone grants no ownership.

Shared mappings support RO/RW and reject executable permissions. They never reinitialize, seal, or restore the shared frame's kernel alias. A RO user view may coexist with a RW view; it is not an immutable snapshot. The backing stays non-executable across every alias. Kernel reads require exclusion of concurrent writers; no borrowed Rust slices are exposed to live shared memory.

Unmap removes the PTE and completes required TLB work before dropping the lease. Failed unmap retains ownership for retry. Closing a handle releases its reference and leaves mappings intact. The last object reference returns the backing frames without waiting for hardware: all mappings must already have retired.

Dropping an unclosed address space deliberately retains its mapping owners. The object arena also retains payloads whose reference counts remain nonzero when the arena is discarded. This prevents a forgotten lease from releasing storage beneath unreclaimed page tables. Such leaks remain allocated and are not a successful cleanup path.

## Receiver capacity and copy delegation

Initial IPC supports at most two delegated capabilities. Each source requires TRANSFER; requested rights must be a subset. DUP remains a separate operation. Delegation copies authority and preserves the original sender handle. Linear move and migration of endpoint RECEIVE or notification WAIT ownership are deferred.

Before publishing Receive, its handle table reserves the requested capacity. Reserved slots cannot be used by insertion or duplication. A move-stable table domain and slot generations reject foreign or stale reservation tokens. Commit installs the staged references without allocation or cloning; failure before validation returns both owners. Unused, canceled, and failed reservations release their capacity explicitly.

Prepared batches hold strong references outside the object arena and are keyed by the sender's generational WaitId. Endpoint state contains only message data and capability counts. Acceptance checks the receiver's reserved capacity before consuming a request, allocating reply authority, or recording receive completion. A mismatched request fails NotDelivered while the receiver remains available for another request.

## Acceptance and cleanup ordering

Ready wait slots may list the sender's Sent completion before the receiver's Received completion. Image code therefore commits all accepted grants before releasing any sender batch, processing task death, or publishing runnable tasks. Received carries the sender TaskId and WaitId so the correct retained batch is consumed. A second acceptance pass handles completions produced while dead tasks detach from channels.

After acceptance, the receiver owns its installed references independently of caller cancellation or termination. Canceling an accepted Call retires its reply token, not the receiver's grants. Before acceptance, cancellation or failure discards only the staged copy; the original sender handle remains intact. All object callbacks finish before handle commit or scheduler wakeup, avoiding arena lock reentry.

## Scope and verification

Native version 0 adds memory creation/map/unmap and capability-bearing Send/Call. Memory creation permits 1..8 pages; map requires READ|MAP plus WRITE for RW. Guest limits remain two tasks, eight objects, eight handles per task, and bounded mappings/tables. These are global/static capacity bounds, not a general per-service quota system.

Host tests cover shared lifetime and cleanup failure, forgotten-reference retention, reservation identity and exhaustion, attenuation, source closure, receiver rejection, and accepted-call cancellation. Combined tests exercise real endpoint, wait, object, and handle APIs.

The guest producer/consumer shares three pages through two reduced-rights capabilities, checks zero initialization and data across a page boundary, closes all memory handles, and removes source mappings before final receiver access. It rejects RW mapping and regrant without authority. A second scenario rejects two capabilities against a zero-capacity receiver, then completes an ordinary message and reclaims all resources. Acceptance is explicitly exercised when the sender's completion precedes the receiver's ready slot. A third scenario closes all handles and attempts a user write through its retained RO mapping; #PF terminates that task and final cleanup releases the backing.

Shared executable pages, buffer loans, shared bulk queues, DMA pins, general quotas, object-wide revocation, linear move, reply capability transfer, SMP, and a stable runtime remain future work. See [status](../status.md) and the [recorded evidence](../validation/2026-10-31-shared/evidence.json).
