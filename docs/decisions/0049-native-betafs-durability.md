# Native BetAFS service and storage barriers

Date: 2026-11-05. Status: experimental implementation.

BetAFS executes as a separate Ring 3 service. The independent engine and codec
algorithms are linked from a verified build copy; they are not added to Ring 0
or modified in their original repository. Kryo supplies execution, memory,
capabilities and the existing block IPC/mediated VirtIO mechanism.

## Assigned storage

The service receives an owned file endpoint, a debug capability and distinct
block read/write endpoints through its immutable startup record. The validator
checks exact roles, kinds, rights, service ID and instance. Namespace clients
receive only their namespace gate; raw block endpoints stay in the service.

The validation volume occupies logical 4096-byte blocks backed by physical
blocks 1 through N-1 of an explicitly assigned scratch image. Physical block 0
retains the existing scratch marker. The adapter checks capacity, offset
addition, request/instance identity, reply shape and exact completion counts.
Only the block driver converts these block IDs into 512-byte sectors.
General device/partition discovery and mount policy remain separate work.

Each block request owns fresh memory backing and a private serialized mapping.
Accepted failure does not authorize reusing backing still referenced elsewhere.
Uncertain transport results poison the session and are not replayed. Writable
VirtIO devices must support FLUSH; the driver checks its hardware completion.

Format is an explicit bootstrap phase. The actual engine refuses nonzero
superblocks; open never falls back to formatting. The regular service mounts
the assigned existing volume. Byte path components use lookup rather than a
UTF-8 conversion. The current engine has regular files and directories, with
no invented symlink representation.

## File lifetime and durability

Open-description gates own inode pins, access/append policy and cursors.
Creation, read/write, positioned I/O, resize and metadata translate to the
engine's existing APIs. Compression uses its current default policy. Directory
cursors own real entry/metadata snapshots; locks use inode identity rather than
path strings. Final close unpins only after accepted work ends. A failed final
unpin retains its closed description and inode ledger until recovery/termination.

Sync Data and Sync All both call FileSystem::sync. A full filesystem barrier is
a valid stronger implementation of the data-only scope. It persists the
engine's data and metadata and waits for the block FLUSH result. Directory
handles support the same barrier. File::flush still concerns runtime buffering
and is not substituted for a storage barrier.

The engine's CommitUncertain/NeedsReopen behavior remains authoritative. Failed
barriers or uncertain writes quarantine the session; subsequent operations and
queued lock requests fail. The service does not infer rollback or replay a
mutation. Replacement/mount routing belongs to the user-space supervisor.

## Native codec build

All four engine codecs are built and executed: Raw, Zstd, LZ4 and OpenZL.
LZ4 is the existing Rust implementation. Zstd and OpenZL compile against the
actual Cryolite C provider. The OpenZL C build disables unused C++ wrappers and
thread dependencies; the Rust engine uses the real C compression/decompression
algorithms. This does not establish a complete libc++ or pthread port.

The build verifies the dirty engine working tree by content, pinned Rust/Cargo,
registry archives and source bytes. Original libc and OpenZL registry packages
are verified before applying owned-copy platform/build changes. Only OpenZL's
cross-build recipe is patched; algorithm source hashes remain unchanged.
The private Cryolite libc overlay and restricted-std compiler admission are
shared with the runtime. Installed source trees and registry sources stay intact.

These programs use a separate ELF link base at 0x10000000 and an explicit
one-MiB initial stack at 0x20000000. Only the top 16 KiB argument image is built
on the kernel stack. Image/guard overlaps are rejected; historical stack and
module defaults remain unchanged. Native image admission and packaging have
explicit larger bounds for these programs. Disassembly rejects MMX.

## Verification boundary

The two-boot harness formats only a newly created owned image, observes real
file/directory barriers, kills QEMU after completion, and reopens that same file
in a separate guest without recreation. The launcher enables barrier events
through startup argument bit 1; ordinary sync performs no debug syscalls.
The kernel pairs service barrier entry
and completion with an increase in successful, acknowledged VirtIO FLUSH
completions. A prior write's barrier cannot satisfy a later no-op sync.

Separate native processes verify that a waiter is blocked in CallExchange
before the lock owner is stopped. Cleanup must release the owner and wake the
waiter, then retire the service, driver, accounts and backing. Codec checks
require the selected codec to match the requested codec before decoding, and
reopen actual engine files stored with all four codecs.

This establishes functional emulator persistence and the implemented command
path. Physical power-loss behavior and hardware performance require hardware
measurements. [Validation](../validation/2026-11-05-durable-files/README.md)
records successful runs, retained failures and exact source/image hashes.
