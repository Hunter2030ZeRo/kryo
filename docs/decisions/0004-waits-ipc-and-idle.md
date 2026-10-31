# ADR 0004: Wait ownership, endpoint IPC, and BSP idle

Date: 2026-10-31

Status: Implemented for bounded BSP scalar execution and exercised by boot-tests.

## Wait lifetime

Core owns a fixed-capacity `WaitTable`. A task has at most one active wait. Each wait has a generation and progresses through Parking, Blocked, and Ready. Completion stores one result; a second completion fails. During Parking the stored result is invisible to ready scans and cannot wake the task. `park` publishes it only after execution handoff. This preserves an event arriving before the task finishes parking.

The current image starts waits after `run_context` has already saved the user context and returned to the boot root/stack. Native calls keep the task Blocked until a result is ready. `settle` copies that result into its saved registers before publishing Runnable, after all object callbacks and locks have ended.

Task termination first detaches requests from every endpoint and notification, closes owned channels, and retires reply tokens. It then abandons its wait before VM/stack reclamation. Generations prevent a delayed event from resolving a replacement task's wait. Slots retire before generation wrap.

Wait IDs are local to one persistent table. Each pending wait belongs to one object. Callers serialize object and wait-table mutation together and retain task resources until detachment. The generic core API documents these obligations; the image enforces them through one operation per native call and one wait field per task. SMP still needs an execution-ownership and locking protocol.

## Endpoint protocol

An endpoint has one receiver-owner task, a bounded FIFO of waiting senders, and a bounded reply table. Requests contain four u64 words. Send completes when a receiver accepts its message. Call remains blocked after acceptance until Reply, cancellation, or endpoint closure. No unbounded mailbox or allocation per request is used.

A reply token names an endpoint-local slot and generation. It retains the caller TaskId and WaitId and binds the receiver. Successful reply, caller cancellation, caller death, and endpoint closure consume it. Repeated or late replies fail. Reply-table exhaustion cannot publish a request without reply capacity.

Cancellation before acceptance removes the pending request and reports NotDelivered. After acceptance it retires reply authority and reports Accepted: the receiver may already have executed the request. This result does not imply rollback. Endpoint closure and owner death resolve waiting receivers, queued senders, and accepted calls once. Remaining capabilities do not reopen the endpoint.

Capabilities validate object kind and SEND/RECEIVE/CONTROL rights before invoking endpoint operations. Notification access uses separate WAIT and SIGNAL rights. A task-control capability with CONTROL can cancel a target wait. User pointers and transferred capabilities are absent from this interface; handle transfer and receiver-capacity reservation remain separate work.

## Notifications

A notification contains one pending bit and one consumer registration. Signal records pending state or completes its registered wait under the same serialized ownership. Wait consumes pending state or registers itself. There is no independent clear operation. Multiple signals coalesce until consumed; a notification is not a completion counter or a message queue.

## CPU idle

With maskable IRQs disabled, the scheduler checks for Runnable work and processes ready completions. If none remain, HAL executes the contiguous `sti; hlt; cli` sequence. STI's interrupt shadow prevents a maskable interrupt from being consumed between enabling interrupts and HLT. This follows the same pattern as the pinned [x86_64 enable_and_hlt API](https://docs.rs/x86_64/0.15.5/x86_64/instructions/interrupts/fn.enable_and_hlt.html).

The BSP timer handler records a monotonic delivery epoch and acknowledges the controller; it does not acquire scheduler/object locks or complete waits. After idle returns with IRQs disabled, image code processes the event and completes the notification. The caller retains the controller mapping, holds no normal locks across idle, and arms an event that can wake it. The pending-before-HLT case is tested as well as subsequent timer delivery.

The guest fixture uses a one-shot hardware count to exercise this path. It is not a calibrated timeout, deadline queue, or power measurement. Normal boot still does not run services. No device IRQ event routing, SMP idle/IPIs, NMI work scheduling, CPU C-state policy, or general tickless timer policy is implemented.

## Verification

Host tests cover early/late completion, one-wait ownership, single completion under cancellation ordering, stale IDs, generation retirement, bounded capacity, FIFO order, reply-token checks, owner/caller death, and notification coalescing.

Both QEMU profiles run eight user-task scenarios: RPC plus Send, queued/accepted cancellation, queued/accepted owner death, explicit closure, and two notification orderings. The RPC validates all 32 payload bytes and rejects a repeated reply. The send-only client cannot receive. Notification cases halt when all remaining tasks block, resume on a real timer IRQ, and complete after deferred signal delivery. A third idle case starts with the timer already queued. Every scenario reclaims its per-task frames and object references.

See [implementation status](../status.md) and [evidence](../validation/2026-10-31-ipc/evidence.json). These tests establish BSP guest behavior, not SMP race freedom or hardware energy savings.
