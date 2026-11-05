# Directory cursors and advisory file locks

Date: 2026-11-05. Status: experimental implementation.

The file-service protocol retains version 1 and extends the existing operations.
Clients and services must use matching definitions; feature negotiation remains
separate work.

| Operation | Authority | Argument / length | Grants / result |
| --- | --- | --- | --- |
| OpenDir (11) | Namespace gate | Zero / path bytes | One READ+MAP loan; returns an open-description gate |
| NextDir (12) | Description gate | Zero / 4096 | One READ+WRITE+MAP loan; returns encoded byte count or zero at EOF |
| Lock (13) | Description gate | Shared=1 or Exclusive=2, optionally TRY=0x100 / zero | No grants; zero on acquisition |
| Unlock (14) | Description gate | Zero / zero | No grants; zero on completion |

WouldBlock=14, Deadlock=15, NotDirectory=16 and IsDirectory=17 extend the service
status values. Namespace and description operations retain their authenticated
gate separation. No request field substitutes for ownership.

## Directory lifetime

OpenDir captures a directory snapshot and creates an independent cursor. A
duplicated description shares its cursor and operation lock; separate opens
remain independent. Ordinary read-only Open also admits directories for
metadata, locking and synchronization. Reading or writing their contents as a
regular file returns IsDirectory; NextDir on a regular file returns NotDirectory.

Each entry is a portable little-endian record: u64 size at byte 0, u64 metadata
attributes at byte 8, u16 name length at byte 16, six zero reserved bytes, then
1–255 name bytes at byte 24. Names cannot contain NUL or slash and cannot equal
`.` or `..`. The existing kind/readonly metadata validation applies. Entries
describe the node itself, including a final symlink when the backend has one.

The provider snapshots and validates the entire bounded record before changing
outputs. The private C record is 280 bytes, aligned to 8: file info, u32 name
length and 255 name bytes. This is not the IPC layout. EOF changes only the
presence result. Malformed or uncertain cursor completions poison aliases;
NextDir is never automatically replayed.

Rust ReadDir owns the description, original path and finished state. DirEntry
owns its raw byte name and metadata snapshot, so it may outlive the iterator.
path joins the original path and checked name; metadata and file_type use the
snapshot without reopening a pathname. EOF or an iteration error finishes the
iterator. The newer Dir traversal API remains unsupported.

## Lock ownership and waiting

Locks are advisory and belong to an authenticated open description over an
actual file/inode. Aliases retain that owner until the last reference closes.
Independent opens conflict according to shared/exclusive mode. Ordinary I/O
does not enforce these advisory locks. Same-mode acquisition is idempotent;
there is no recursive acquisition count.

A conflicting shared-to-exclusive upgrade retains the shared lock and returns
Deadlock for a blocking request or WouldBlock for a try request. Downgrading
releases exclusive ownership without dropping the description. Try operations
never queue. Rust exposes contention as TryLockError::WouldBlock.

A blocking request retains its single-use reply token while the service keeps
receiving. FIFO ordering is per inode: a queued writer prevents later readers
from bypassing it, and adjacent shared requests may proceed together. Other
inodes can progress. The client makes one RPC and does not poll for acquisition.

Before replying, the service tentatively installs ownership. Failed/canceled
reply delivery rolls it back and advances the queue. Final GateClosed releases
owner state after accepted kernel loans retire. Pending-call cancellation may
be discovered only when replying or receiving the final close event; no immediate
cancellation notification is claimed. Storage/session failure rejects pending
lock requests immediately rather than leaving them asleep indefinitely.

The volatile peer retains its bounded test inventory. The BetAFS service uses
fallible growing description and pending-lock storage. Both are covered by
[validation](../validation/2026-11-05-durable-files/README.md).
