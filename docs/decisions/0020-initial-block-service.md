# Initial block service

Date: 2026-11-02. Status: implemented configured service; VFS integration pending.

## Ownership and startup

An explicitly enabled initial block service now uses the same Objects, Tasks and
IPC runtime as the normal supervisor. It remains a separate Ring 3 process.
Boot wiring prepares that process suspended, obtains its ProcessId, creates the
process-bound transport and installs a version-one startup receipt before making
the thread Runnable. It does not rebind a supervisor's device by copying a handle.
Existing spawn paths use the same suspended preparation followed by publication.

The driver has separate READ, WRITE and CONTROL gates, each an endpoint bound to
its initial receiving thread. Clients receive SEND alone. Gate possession carries
authority; claimed IDs and request fields cannot add write or stop permission.
The normal service rejects Stop. The isolated boot-test graph explicitly allows
a control client to request orderly stop.

Driver and clients bind resource accounts before image allocation and execution.
Each account has eight shared pages, 64 handles, eight waits, one thread and
per-process bounds of 64 mapped pages/32 tables. Three boot-created endpoints
share a three-payload channel account. These are configured service bounds, not
global process or thread limits.

## Block protocol version 1

Control messages carry four little-endian u64 words:

| Request word | Meaning |
| --- | --- |
| 0 | Version in the upper 32 bits; operation in the lower 32 |
| 1 | Service instance |
| 2 | 4096-byte block index, or zero for non-data operations |
| 3 | Nonzero request correlation ID |

Operations are Info=1, Read=2, Write=3, Flush=4 and Stop=5. Read and Write carry
exactly one delegated Memory capability; the first page is the entire fixed-size
buffer. There are no wire pointers or caller-supplied VirtIO commands. Other
operations carry no capability. Replies are request ID, status, value and instance.
Info returns the 4096-byte block count with bit 63 indicating a read-only device.
Successful data operations return 4096; Flush and Stop return zero.

The READ gate allows Info/Read. The WRITE gate additionally allows Write/Flush.
Only the test CONTROL gate allows Stop. Validate version, instance, operation,
correlation ID, block bounds and capability count before mapping. Read requires
READ|WRITE|MAP on its output memory; Write requires READ|MAP on its input. The
capability must have TRANSFER at the sender, but the service need not receive
that delegation right. Status values distinguish malformed, denied, range,
read-only, I/O, uncertain, unavailable and stale requests. The fixed-width enums
and decoder live in interfaces/block.rs.

IDs correlate replies and instances reject stale traffic; neither provides
deduplication or exactly-once writes. A successful Stop reply acknowledges the
request. Process exit and kernel retirement establish completion of device cleanup.

## Loans, DMA and cancellation

The single-threaded service maps one peer page at a private loan address using raw
accesses. It copies input into its own transport memory, or copies completed
output back from that memory. The existing private DMA queue never references
peer physical frames. Full used length and device status are checked. Write and
Flush errors do not become successful commits; possible publication errors are
reported uncertain, and private buffers are reset before reuse.

Retire the CPU loan and close every delivered handle before replying. Rejection,
two-grant malformed input and fire-and-forget messages follow the same cleanup.
Failure to unmap is fatal to the service and leaves VM ownership for process
retirement. InvalidReply after cancellation is nonfatal to the driver.

Cancellation alone does not release a loan or roll back a write. Clients must
not reuse the buffer before confirmed service completion. In the configured
single-threaded service, a subsequent successful Info request establishes that
the preceding accepted request finished its loan cleanup. This is a sequencing
barrier for that instance, not a general multi-threaded loan-return protocol.

## Scheduling and recovery

The driver uses ReceiveFor to rotate three gates with one-millisecond waits.
Device completion still uses transport polling and short timed sleeps. There
is no busy spin, but periodic idle wakeups remain; wait-any and device IRQ
completion are needed before making efficiency claims.

Normal boot retains the driver after its configured clients exit, alongside the
supervisor. Boot tests use a control client to stop it and verify all accounts,
objects, waits, channels and frame owners retire. Kernel device retirement occurs
before execution-resource reap. Failed cleanup retains resources and normal idle
arms a retry deadline at the earlier of one millisecond and the next IPC deadline.
It does not depend on unrelated supervisor wakeups.

This initial graph is enabled only with the signed disposable-disk test setup.
General driver discovery, external service publication, automatic device
reassignment/restart, storage IRQs and a persistent production configuration
format remain pending. No filesystem or std implementation is added here.

See [validation](../validation/2026-11-02-block-service/README.md),
[private transport](0018-mediated-virtio-block-transport.md) and
[native runtime direction](0019-native-runtime-and-standard-libraries.md).
