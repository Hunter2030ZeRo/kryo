# Fallible provider stdio

Date: 2026-11-04. Status: implemented static provider correction.

The native Talc allocator returns allocation failure rather than terminating the
process. The pinned provider must preserve that behavior in FILE construction
and buffering before service-backed stdio can be connected.

Allocate FILE storage before Open. A failed allocation returns ENOMEM without
opening a descriptor; failed Open releases the unpublished storage and preserves
the operation's error. Placement construction happens only after both steps
succeed. This avoids requiring a fallible Close merely to recover from FILE
allocation failure after a successful Open.

Buffer allocation checks addition overflow and the allocator result before
forming the pushback/data pointers. Failure leaves allocation state retryable,
reports ENOMEM and sets the stream error indicator. A partial fread followed by
an error returns the number of complete elements already read rather than zero.
Read returns consumed pushback as a short success before another operation can
fail, so that prefix is included in the next fread error/EOF boundary too.
The patch changes only these paths in the fixed upstream source revision.

The native probe commits most of its real private-memory account in bounded
chunks and drains the existing heap slack, then checks fputc/ungetc
failure, frees its allocations and verifies pushback recovery. It preserves
errno and the floating-point environment and leaves no dirty stdout data.
The kernel requires one event per process and complete backing retirement.
This is memory/error-path validation: file hooks still reject unsupported I/O.

A separate host harness compiles the actual patched function bodies with mock
allocator/Open/Read dependencies. Twenty-five cases run at O0 and O2 with address and
undefined-behavior sanitizers. It checks FILE allocation before Open, failed
Open cleanup, buffer overflow/NULL handling and partial-read error/EOF results.
These mock cases do not establish a running filesystem or successful native
fopen. Pinned inputs and extracted bodies are recorded with their hashes.

See the [validation record](../validation/2026-11-04-stdio/README.md).
