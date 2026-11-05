# Native std file operations

Date: 2026-11-05. Status: experimental implementation.

Connect Rust File and OpenOptions directly to the existing service-backed C
descriptor hooks. Reuse their pinned descriptions, per-operation lock, reduced
gate authority, shared cursor and accepted-error poison state. No new file wire
operation, kernel filesystem, receiver thread or duplicate service gate is
needed. BetAFS remains an independent Ring 3 service.

File owns an OwnedFd. Cloning duplicates a local descriptor with CLOEXEC from
descriptor 3 upward; aliases share the server's open description. A separate
Open creates an independent cursor. Consuming File/OwnedFd/raw conversions move
ownership without duplicating or closing it. Drop retains the existing final
reference and closure rules.

Translate Rust access/creation options into the owned Cryolite C flags. Access
is required; creating/truncating requires write or append, and append+truncate
is invalid unless create_new overrides truncation. Unix OpenOptionsExt forwards
mode and supported custom flags. Mode enforcement belongs to the namespace
service; this profile has no umask or production permission policy.

Reject paths beyond 4096 bytes and interior NUL before calling the C layer.
Construct the terminated byte path with fallible allocation. The caller's
namespace grant determines resolution; this does not install global mounts,
cwd or host paths. Read/Write preserve bounded short completions, allowing the
standard library's read_to_end/read_exact/write_all loops to span loan pages.
Use the upstream initialized read-buffer and vectored fallbacks, validating
the returned count before advancing a Rust buffer. This is not native vectored
I/O. Seek rejects unrepresentable Start offsets before changing the cursor.

Metadata, directories, positioned I/O, length mutation, file locks, permission
changes and durable synchronization remain unsupported. In particular, do not
emulate read_at/write_at or size queries with separately locked seek/restore
calls: cloned descriptors share their cursor. File::stream_len returns an error
without moving it. File::flush succeeds because File adds no runtime buffer;
sync_all/sync_data must not fabricate durable completion.

The native probe uses the existing volatile file peer and requires client exit,
peer exit, final capability closure, empty wait/channel registries and complete
resource/backing retirement. See the [validation record](../validation/2026-11-05-std-files/README.md).
This establishes basic std file transport, not a mounted filesystem, BetAFS or
storage durability.
