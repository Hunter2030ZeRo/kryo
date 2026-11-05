# BetAFS service boundary

BetAFS remains an independent Ring 3 filesystem service. Kryo supplies memory,
capabilities, execution and IPC; it does not link the filesystem engine into
Ring 0. This directory records the adapter boundary. There is no executable
BetAFS service or mounted filesystem here.

## Block transport

The current `betafs_block::BlockDevice: Send` interface uses:

```rust
fn block_count(&self) -> u64;
fn read_block(&self, id: u64, block: &mut [u8; 4096]) -> std::io::Result<()>;
fn write_block(&mut self, id: u64, block: &[u8; 4096]) -> std::io::Result<()>;
fn sync(&mut self) -> std::io::Result<()>;
```

The version-one native block protocol already provides Info, Read, Write and
Flush with fixed 4096-byte blocks. Pass a BetAFS block ID unchanged in the IPC
request. The device transport alone converts it to eight 512-byte sectors with
checked arithmetic. Multiplying in both layers would address the wrong data.
Validate instance, request ID, status and the complete result: 4096 for data,
zero for Flush. Capacity is a block count; Info bit 63 is the read-only flag.

Successful sync must make every preceding write durable. A submission, IPC
reply or interrupt alone is insufficient; the current writable VirtIO service
requires FLUSH support and checks its completion. QEMU tests establish this
command path, not physical power-loss durability. Failed writes/barriers can
leave partial or uncertain media. Preserve the engine's CommitUncertain and
NeedsReopen behavior rather than turning them into a retryable success.

Read-your-writes and accepted buffer ownership must survive error paths. An
accepted cancellation does not roll back a write or end its loan. Fresh backing
per request avoids reusing RAM still owned by a service; an alternative requires
an explicit completion barrier. Never send Rust pointers or std::io::Error
representations over IPC. Read and write grants carry only their required
memory rights. Resource accounting follows actual retirement.

The trait requires Send, not Sync. A serialized service can retain its block
session in RefCell while read_block uses &self; parallel access needs a separate
synchronization contract. No additional kernel filesystem interface is required
by this storage boundary.

## File service

The native regular-file protocol can be translated to the existing engine API:

| Native operation | BetAFS API |
| --- | --- |
| Open lookup/create | lookup, create_file_with_attributes, metadata |
| Read | read_at |
| Write | write_at |
| Truncate on Open | truncate |
| Open-description lifetime | pin_inode, unpin_inode |
| Storage barrier | FileSystem::sync |

Resolve byte components with lookup and *_at methods; converting every native
path through UTF-8 resolve would reject valid byte names. The service owns
cursor, append serialization and access checks. Stored UID/GID/mode fields do
not authorize the requesting client. One accepted four-KiB Write fits the
engine's 64-KiB transaction bound. Separate Open calls have separate cursors;
duplicated gates retain the same description. Final gate closure permits unpin
only after accepted work finishes; failed last-unpin retains its owner.

File and block messages have different word layouts and status registries.
Translate them explicitly. The current file protocol does not implement
stat, directory operations, fsync, inherited streams or a complete VFS.
Adding those services is separate from validating this boundary.

## Native execution gates

The engine keeps std. The current Cryolite target is a freestanding bootstrap
with std=false; a native Rust standard-library platform and the codec C/FFI
runtime remain required. The inspected engine unconditionally compiles its Unix
FileDevice backend using std::os::unix::fs::FileExt. A future native build needs
a compatible Unix platform surface or separation of that host-image backend.
The current codec manifest also includes Zstd, LZ4 and OpenZL unconditionally;
selecting Raw at runtime does not skip their build dependencies.

The configured Ring 3 block IPC service exists, but its transport deliberately
requires the fixed CRYOLITE-BLOCK-PROBE-V1 marker in block zero. It admits disposable
scratch disks, not arbitrary BetAFS-formatted media. General device assignment
and image admission are still needed. Compatibility verification must preserve
that guard rather than overwrite a filesystem superblock with the test marker.

## Verification scope

`tools/check_betafs.py` checks an explicitly selected local engine tree without
modifying it. Its isolated host harness links the actual engine and native wire
definitions, exercises real filesystem operations through a host block-transport
model, and records source/lock hashes. Native QEMU block-service tests are a
separate result. Neither result alone establishes BetAFS execution inside Kryo.
See the [validation record](../../../docs/validation/2026-11-05-betafs-boundary/README.md).

[Runtime direction](../../../docs/decisions/0019-native-runtime-and-standard-libraries.md),
[block IPC](../../../docs/decisions/0020-initial-block-service.md) and
[file descriptions](../../../docs/decisions/0040-service-backed-files.md)
define the existing contracts. BetAFS is not rewritten as no_std or copied into
this repository.
