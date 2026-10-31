# Interface design

The public service ABI is not stable. Experimental task calls are defined below; the remaining sections list decisions required before publication. Ownership and concurrency rules are specified in the [design contract](design-contract.md).

## Experimental task calls

`interfaces/src/syscall/` defines version 0, used by `boot-tests`. Entry is `int 0x80`; RAX selects a call, RDI and RSI hold arguments. Return RAX is zero or a negative error; RDX holds the result. IPC uses R8..R11 for four u64 words. Receive and successful Call replace those message registers; other general registers are preserved. The separate RDX result preserves all 64 bits of a handle or reply token.

| Number | Call | RDI | RSI | RDX on success |
| --- | --- | --- | --- | --- |
| 0 | Yield | Unused | Unused | 0 |
| 1 | Exit | Exit status | Unused | Does not resume |
| 2 | TaskStatus | Task-control handle | Unused | Task status |
| 3 | TaskStop | Task-control handle | Exit status | 0; self-stop does not resume |
| 4 | Close | Handle | Unused | 0 |
| 5 | Duplicate | Handle | Reduced rights | New handle |
| 6 | Send | Endpoint handle | Unused | 0 after receiver acceptance |
| 7 | Receive | Endpoint handle | Unused | Reply token, or 0 for Send |
| 8 | Call | Endpoint handle | Unused | 0 after reply |
| 9 | Reply | Endpoint handle | Reply token | 0 |
| 10 | EndpointClose | Endpoint handle | Unused | 0 |
| 11 | Wait | Notification handle | Unused | 0 |
| 12 | Signal | Notification handle | Unused | 0 |
| 13 | CancelWait | Task-control handle | Unused | Delivery state |

TaskStatus requires INSPECT; TaskStop and CancelWait require CONTROL. Duplicate requires DUP and a subset of the original rights. Rights bits are INSPECT=1, CONTROL=2, DUP=4, SEND=8, RECEIVE=16, WAIT=32, SIGNAL=64. Send/Call require SEND, Receive/Reply require RECEIVE, and EndpointClose requires CONTROL. Wait and Signal require their matching rights. Handles are process-local and opaque; zero is invalid. Close removes one reference, not all authority to an object. A task-control object can outlive its task and report Exited without keeping its VM or stack alive.

Errors are InvalidCall, InvalidHandle, PermissionDenied, WrongType, OutOfResources, InvalidArgument, PeerClosed, Cancelled, and InvalidReply. Unknown call numbers and rights bits are rejected. These calls contain no user pointers. Shared buffers, user-copy recovery, capability transfer, and timed waits are not implemented.

Send and Call carry R8..R11; Receive returns them. Reply supplies its response in R8..R11 and its endpoint-local token in RSI. Tokens are bound to the receiver and caller wait and are single-use. Each endpoint has one receiver-owner task in this stage; RECEIVE transfer to another process is unsupported.

Cancellation and endpoint closure return delivery state in RDX: 0 means not accepted, 1 means accepted and possibly executed. After acceptance, a failure does not promise rollback. CancelWait only targets an active wait and requires task-control authority. Already completed operations keep their first result; late replies fail. Wait consumes one coalesced pending notification or blocks; repeated signals before consumption do not count separate events.

The bootstrap `run_user` helper retains its earlier exit-only int 0x80 behavior for the memory/entry regression cases. `run_context` uses the version-0 dispatcher. This mode distinction is internal test machinery and must be removed before a public runtime ABI is published.

## Request paths

Use kernel IPC for small control requests and capability transfer. Use registered shared memory and bounded request/completion queues for bulk data. Measure registration, mapping, copying, and wakeup costs before selecting a path for a workload.

## ABI decisions

| Area | Required definition |
| --- | --- |
| Representation | Field widths, alignment, byte order, versioning, lengths, and validation |
| Handles | Object types, rights, lookup, duplication, transfer, close, generations, and reuse |
| Shared buffers | Object identity, offset, length, rights, loan duration, and bounds checks |
| Queues | Producer/consumer count, capacity, full-queue behavior, ordering, and notification |
| Async requests | Request IDs, completion, cancellation, timeouts, retry, and duplicate handling |
| Services | Stable identity, instance generation, failure, and reconnection |
| DMA | Pinning, IOMMU mappings, device quiescence, revocation, and late completion |
| Files and paging | VFS handle lifetime, file-backed pages, dirty data, flush, and service failure |

Use explicit wire representations. Do not pass Rust references or trait objects across an ABI. Snapshot validated shared metadata before using it.

The control path currently implements 32 inline bytes. The proposal for two transferred capabilities, discovery, and a stable register layout still requires definition before ABI publication.

## Replacement and recovery

Orderly replacement stops admission, resolves in-flight work, quiesces devices when needed, transfers state, grants authority to the new generation, and reconnects clients.

Crash recovery first establishes instance termination and resource/device safety. It then restores available state and starts a new generation. Accepted requests may have uncertain outcomes; retries require defined idempotence or deduplication.

Kernel mechanisms provide handles, mappings, IRQ authority, waits, and termination. Service protocols define operation-specific recovery.

## Measurements

Compare direct calls, small IPC, and shared queues. Record p50/p99 latency, cycles per request, bytes copied, memory use, context switches, wakeups, idle CPU, energy per operation, recovery time, and lost/retried requests.

Keep QEMU functional results separate from hardware performance and power measurements.
