# Descriptor metadata and positioned I/O

Date: 2026-11-05. Status: experimental implementation.

Extend the existing file-service protocol, retaining version 1 and operations
1–4. These additions require the matching provider and service; there is no
feature negotiation or automatic compatibility with an older server.

| Operation | Argument / length | Request grants | Successful response |
| --- | --- | --- | --- |
| ReadAt (5) | File offset / bytes ≤4096 | READ+WRITE+MAP memory | Completed bytes |
| WriteAt (6) | File offset / bytes ≤4096 | READ+MAP memory | Completed bytes |
| Info (7) | Zero / zero | None | Size in word 2; attributes in word 3 |
| SetLen (8) | New byte length / zero | None | Zero |
| Sync (9) | All=0 or Data=1 / zero | None | Zero after the requested barrier |

At operations use loan offset zero and never modify the shared cursor. Signed
64-bit positions remain the admitted range. EOF reads return zero without
adding a large requested length to the offset. Writes validate their bounded
range before publication, return actual partial progress and zero-fill gaps.
The existing description operation lock still protects the reusable loan window.

Positioned writes follow POSIX behavior. WriteAt honors its explicit
offset even when APPEND is set. Ordinary Write still selects EOF atomically.
APPEND is an open-description I/O policy, not append-only security authority.
Any future append-only restriction requires a separately enforced operation
policy. SetLen requires writable access, also works on a writable append
description, preserves cursors and initializes growth, including shrink/regrow.

## Metadata

Info returns the object's size, type and genuine service readonly property.
Its attributes are type in the low 32 bits and flags in the high 32 bits.
Known types are Regular=1, Directory=2 and Symlink=3; READ_ONLY=1 is the only
flag. Unknown types/flags and sizes beyond the signed position range are invalid.
Only successful Info replies may use word 3. Error replies still require zero
value/attributes and no capabilities; every other successful reply keeps its
existing shape. Malformed replies poison all local aliases before output is
published.

The private C record is 16 bytes: u64 size, u32 kind, u32 flags. It is not the
proposed full C stat layout and is not copied as a wire struct. Rust Metadata
uses real length/type/permissions snapshots; timestamps remain Unsupported.
Opening a mutable file read-only does not make its object permissions readonly.
Changing a Permissions snapshot does not change service policy; permission
mutation remains unsupported. Unix MetadataExt/PermissionsExt and path stat,
directories, namespace metadata and symlink resolution remain separate work.

File::stream_len uses one Info request without seeking. Stable FileExt
read_at/write_at and the upstream exact/all loops use the new operations.
The pinned Rust stream_len API itself still requires its upstream feature gate.

## Synchronization and failure

Sync Data must persist file data and metadata required to retrieve it, including
size/allocation changes. Sync All additionally persists other changed file
metadata. Neither operation automatically commits the containing namespace.
The service must wait for its required lower-level writes and storage barriers
before returning success. A reply, File::flush or RAM copy is not such a barrier.
The current volatile fixture returns Unsupported for both scopes. BetAFS's
future Ring 3 adapter must implement the actual storage contract.

All operations retain existing descriptor pins, grant/loan retirement and
accepted-error poisoning. Outputs are assigned only after complete reply
validation. Mutating calls are not automatically replayed after uncertainty.
Ordinary quota/admission failure is still an error; concurrency needs sufficient
wait credits for the relevant IPC, locks and joins.

The [validation record](../validation/2026-11-05-file-ops/README.md) distinguishes
host boundary injection, actual service behavior and the native occupied-credit
rejection/recovery case. No kernel filesystem or independent BetAFS source was
changed.
