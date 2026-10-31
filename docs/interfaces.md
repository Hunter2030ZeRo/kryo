# Interface design

The user-visible ABI is not defined yet. This document lists the decisions required before publishing it. Ownership and concurrency rules are specified in the [design contract](design-contract.md).

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

The first control-path proposal is synchronous endpoint IPC with up to 32 inline bytes and two capabilities. These limits and the register layout require confirmation before ABI publication.

## Replacement and recovery

Orderly replacement stops admission, resolves in-flight work, quiesces devices when needed, transfers state, grants authority to the new generation, and reconnects clients.

Crash recovery first establishes instance termination and resource/device safety. It then restores available state and starts a new generation. Accepted requests may have uncertain outcomes; retries require defined idempotence or deduplication.

Kernel mechanisms provide handles, mappings, IRQ authority, waits, and termination. Service protocols define operation-specific recovery.

## Measurements

Compare direct calls, small IPC, and shared queues. Record p50/p99 latency, cycles per request, bytes copied, memory use, context switches, wakeups, idle CPU, energy per operation, recovery time, and lost/retried requests.

Keep QEMU functional results separate from hardware performance and power measurements.
