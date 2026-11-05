# Service-backed file descriptions

Date: 2026-11-05. Status: implemented experimental native contract.

## Ownership

The file service owns each open description: access mode, append status and
current offset. A native EndpointGate identifies that description. Duplicated
capabilities share it; separate Open calls create independent offsets. The
kernel authenticates gate labels, not file IDs supplied in request words.
The namespace gate authorizes lookup within its server-defined namespace. It
does not grant access to a global namespace, arbitrary paths or other services.
The current provider receives an optional namespace EndpointGate with SEND in
startup role 2. Role 1 remains diagnostic authority.

The libc descriptor table grows fallibly. Descriptor lookup uses a bounds-checked
array; final-record unlink still scans a cold list. Table storage retires with
its final record or operation pin. It reserves local publication state
before remote Open and publishes only after validating the complete reply and
gate kind/rights. Open allocates from descriptor 3 in this profile. Descriptors 0, 1 and 2
are initially absent, but explicit duplication can populate them; inherited
streams remain separate work. A short table lock protects publication
and operation pins; it is not held across IPC. Each live or pinned open description owns
a reusable mapping slot and an operation lock, shared by its descriptor aliases. Close removes the
numerical descriptor immediately; an in-flight operation retains the old record
and gate, independently of numerical descriptor reuse. Final description release closes
the gate. The server releases description state on GateClosed only after its
accepted work no longer uses it.

The static runtime owns scratch virtual addresses
`[0x200000000000, 0x210000000000)` above the private VM reservation arena.
Applications must not install mappings in that runtime range. Slots are reused
after final operation retirement; repeated RPCs do not consume new virtual
addresses or an unbounded set of intermediate page tables. This is a runtime
address policy, not new kernel allocation authority or a stable native ABI.

## Wire version 1

Each request has four u64 words: `[header, request_id, argument, length]`.
Header is `(1 << 32) | operation`; request IDs are nonzero and must not wrap.
The gate supplies instance identity. A response is
`[request_id, status, value, 0]`; error responses have value zero and no grants.
The Rust module and C protocol header define the same independent wire values.
C public open flags are checked and translated, not forwarded as native rights.

| Operation | Argument | Length | Request grants | Successful response |
| --- | --- | --- | --- | --- |
| Open (1), namespace only | Flags in low 32 bits; creation mode in high 32 bits | Path bytes, 1..4096 | One READ+MAP memory object | Value zero and one SEND+DUP+TRANSFER EndpointGate |
| Read (2), description only | Offset in loan page | Bytes, full range within one page | One READ+WRITE+MAP memory object | Completed byte count, including zero at EOF |
| Write (3), description only | Offset in loan page | Bytes, full range within one page | One READ+MAP memory object | Completed byte count |
| Seek (4), description only | Signed i64 displacement as u64 bits | SET=0, CUR=1, END=2 | None | Nonnegative signed-64-bit position |

Open paths are byte strings without embedded NUL. The service snapshots path
bytes before resolving them; production path traversal, mounts, CWD and symlink
policy remain separate work. The fixture recognizes exact paths only. Mode is
limited to 07777 and is zero without CREATE. Native flags select READ_ONLY=0,
WRITE_ONLY=1 or READ_WRITE=2, with CREATE=0x10, EXCLUSIVE=0x20,
TRUNCATE=0x40 and APPEND=0x80. Unknown flags, invalid access combinations,
EXCLUSIVE without CREATE and read-only TRUNCATE are rejected. The provider
rejects recognized unsupported C flags rather than dropping them. CLOEXEC is
recorded locally; no exec or descriptor inheritance is implemented.

The service validates capability type/rights, grant count and the complete
range before access. The current runtime creates fresh one-page backing for
each RPC at its reusable slot. Shared bytes are accessed through volatile raw
pointers without constructing incompatible cross-process Rust references.
Write input is copied to a private snapshot before updating service data.
All CPU loan access ends and the service mapping/grant retires before reply.
This does not establish safe DMA retirement or asynchronous loan cancellation.

Calls have no client timeout and no automatic replay. An accepted native IPC
failure makes the description unusable for later I/O while preserving the first
error; Close remains available. This prevents a later fflush or process-exit
flush from replaying dirty bytes whose server-side execution is unknown.
Undelivered failures remain retryable. A transport failure after
acceptance does not imply that the operation was undone. Fresh backing is never
reused for a subsequent request; a retained service capability keeps old RAM
alive even when the client drops its view. Generic services must retain their
own accepted-operation owners if they introduce asynchronous work.

Append chooses EOF and writes in one server operation. Each provider Read/Write
hook issues one bounded RPC and may return a short count; mlibc loops as needed.
This does not make an entire multi-RPC fwrite atomic. Services return a successful
byte count for partial progress. If a backend cannot determine write progress,
it must retain the accepted work and refuse further I/O on that description;
a retryable error must not conceal unknown execution. Seek uses checked signed
arithmetic and leaves state unchanged for negative or unrepresentable results.
An IPC reply and fflush do not establish durable storage completion.

## Validation backend and limits

The optional libc probe uses mode 4 of the separate Ring 3 service-peer image.
Its bounded 16-KiB memory backend has one mutable file and a read-only file,
with eight description slots as fixture policy. It is neither a filesystem
implementation nor a replacement for BetAFS. Normal supervisor startup receives
none of this fixture authority. The generic provider descriptor table has no
such eight-file limit.

The native fixture exercises C FILE APIs across separate address spaces,
shared description offsets through duplicated gates, authority/range checks,
EOF, append, truncation and closure after client process retirement. The service
waits without a polling deadline and exits only after namespace and file gates
close. Trusted success requires both process exits and complete task, object,
account, heap and frame retirement. Host source-body tests cover concurrency and
fault paths separately; mock replies are not native filesystem evidence.

This remains the ANSI/static provider profile. Public POSIX file functions,
stat, fcntl, inherited streams, terminals/PTY, readiness, production VFS, mounted
BetAFS, durable flush and recovery remain separate deliverables. Current costs
include per-RPC shared-page allocation, mapping and bounded copies, and a
single receiver per service. Measurements and later optimizations must preserve
these authority and lifetime rules; no hardware performance result is claimed.
