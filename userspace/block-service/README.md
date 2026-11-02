# Native block service

A single-threaded Ring 3 service owns the mediated VirtIO transport. Typed startup
grants provide separate read, write and control admission endpoints. Endpoint
capabilities select policy; message fields cannot grant write or stop authority.

Version-one requests carry four u64 words: version/opcode, instance, 4096-byte
block index and nonzero request ID. Read/write carry exactly one memory capability
covering the first page. Replies are request ID, status, result and instance.
Info returns the block count, with bit 63 indicating a read-only device. Stop is
accepted on the control gate only when startup argument 1 enables test teardown.

The service validates and maps the client page, copies through its own transport
buffer, then unmaps and closes every received grant before reply. No client
descriptor or raw address reaches hardware. An accepted timeout is uncertain;
the caller must not infer that its request or loan ended. Native transport errors
reset before reuse, and failed writes are never automatically retried. A late
reply after cancellation is discarded without terminating the service.

The initial driver requires the explicitly enabled signed scratch device. It is
not a mounted filesystem, general device discovery policy or BetAFS adapter.
Completion polling and timed round-robin gate admission remain bootstrap limits.
