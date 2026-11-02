# Mediated VirtIO transport and Ring 3 block commands

Date: 2026-11-02. Status: implemented validation path; persistent block service pending.

## Boundary

General drivers remain in Ring 3. The native `virtio-block` image selects block
features, constructs requests, interprets device status, and implements read,
write and flush. HAL implements modern PCI transport operations. Core validates
buffer plans and serializes private split queues; image wiring retains device,
DMA and memory owners. Ring 0 does not parse block opcodes or interpret a flush
as a filesystem commit.

The current trusted image inventory admits modern VirtIO block `1af4:1042` and
allows VERSION_1, RO and FLUSH. This is one bootstrap binding, not general driver
selection. The existing Device kind now permits CONTROL for transport start/reset.
Each binding also checks its original ProcessId: a copied handle cannot rebind
the device to a different process. Closing one handle does not retire the binding;
process retirement or explicit reset governs accepted work.

## PCI transport

Use the standard PCI configuration-access capability to reach bounded common,
notify, ISR and device regions. A transaction holds the HAL configuration lock
and excludes local IRQ reentry. Byte/word/dword access matches the field width;
command writes do not overwrite PCI status W1C bits. No BAR sizing, user MMIO map
or arbitrary configuration write is exposed.

Initialization follows reset completion, ACKNOWLEDGE, DRIVER, feature discovery,
FEATURES_OK acceptance, queue discovery/configuration, then DRIVER_OK. Early
queries are rejected. Failed partial initialization requires reset. PCI INTx is
disabled; enabled MSI/MSI-X at claim is rejected. Completion is polled by the
driver, which sleeps between unsuccessful polls. This path does not establish
storage IRQ delivery or optimized idle behavior.

Configuration-window access is a bootstrap mechanism with more transactions
than direct MMIO. Only conventional segment-zero configuration, split queues,
one instance and the supported feature subset are implemented. Packed queues,
indirect descriptors, event-index negotiation and multi-queue operation remain
pending. Strict duplicate required capabilities are rejected by this bootstrap
parser. Device status and bounded configuration reads can report errors.

## Private DMA

A binding owns three contiguous frames. The first holds an eight-entry split
queue; the other two hold at most 8192 packed payload bytes. At most three
user records describe memory-object offsets, byte lengths and device-write flags.
They cannot specify physical addresses, links or indirect tables.

Snapshot and validate the entire record list before access. Check count, flags,
nonzero lengths, overflow, object bounds, disjoint ranges and readable-before-
writable ordering. Hardware descriptors use only the retained private DMA base
plus validated packed offsets. Copy readable input before publication and zero
device-writable storage. User changes after acceptance cannot redirect DMA or
change completion destinations.

One ticket owns the outstanding chain and a strong reference to its original
memory object. Release ordering precedes availability and the doorbell. Acquire
ordering precedes completion data reads. Reject malformed used indices, head IDs
and lengths and poison the queue until hardware reset. Copy device-writable
ranges back only after the device returns the chain. The Ring 3 block driver
checks full expected completion length and status separately.

This contains driver-supplied addresses for a compliant VirtIO device without
an IOMMU. It does not isolate malicious hardware. No raw DMA domain, user queue,
shared-page pin or general DMA quota is added. The bootstrap allocation and single
outstanding request are explicit per-binding bounds, not system-wide limits.

## Native calls

All calls retain experimental ABI version 0. Errors use the existing negative
status register; successful values return in RDX.

| Call | Input | Result |
| --- | --- | --- |
| VirtioInfo 49 | Device READ; RSI selector 0 features, 1 first device-config u64 after start, 2 software queue bound | u64 |
| VirtioStart 50 | Device CONTROL; RSI selected features | zero |
| VirtioSubmit 51 | Device WRITE; RSI READ+WRITE Memory; RDX record offset; R8 count 1..3 | nonzero ticket |
| VirtioPoll 52 | Device READ; RSI ticket | zero pending; bit 63 set plus u32 used length on completion |
| VirtioReset 53 | Device CONTROL | zero pending; one after reset and preparation for feature negotiation |

Each little-endian record is 16 bytes: offset u64, length u32, flags u32. Flag 1
means device-writable; zero means device-readable. A second submit while busy,
stale ticket or malformed record returns InvalidArgument. A failed notification
may follow availability publication; a submit error does not promise no device
work occurred. Reset before recovery rather than blindly retrying a write.

## Retirement and restart

Process death blocks new operations. Reset must report status zero before old
queue access is retired, original memory references are dropped, or DMA RAM is
returned. Failed cleanup retains all remaining owners for retry. Drop performs
no device wait and does not recycle active backing. Successful close restores
the original PCI command and releases the device claim. A temporary initializer
borrows VM policy only while creating RW+NX buffer aliases; retained hardware
ownership does not borrow process or VM metadata.

Explicit reset discards pending completion after hardware stops and prepares a
new negotiation phase. Software queue indices reset while ticket generations
continue. An old poll cannot consume a replacement request. Two validation
instances also claim, reset, retire and reclaim the same PCI function in turn.

## Validation and filesystem integration

Default images do not run the block probe. `BLOCK_PROBE=1` adds an explicit boot
marker; the driver then verifies a disposable disk signature before writing.
The smoke runner creates its own signed file and checks the entire expected
image after stopping its emulator. Scratch block 1 is the only successful write.
Read-only mode requires an actual device error and unchanged bytes. This fixture
has no filesystem and does not replace BetAFS.

BetAFS remains a normal Ring 3 `std` service through the future Cryolite Rust
target and standard-library platform support. Its adapter needs 4096-byte blocks,
read-your-writes and actual durable flush. Native block commands are now exercised;
the BetAFS engine, VFS, file operations and native standard-library ports are
still pending. See [runtime direction](0019-native-runtime-and-standard-libraries.md)
and [validation](../validation/2026-11-02-block/README.md).

The mechanism and reset ordering follow [VirtIO 1.2](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html).
