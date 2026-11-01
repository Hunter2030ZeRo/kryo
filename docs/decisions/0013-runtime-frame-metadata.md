# Runtime frame metadata

Date: 2026-11-01

## Decision

Size physical-frame allocation metadata from the normalized boot memory plan.
The image reserves its backing before activating the frame pool or global heap.
Production uses external bitmap and summary buffers; bounded inline storage
remains available for deterministic host tests. This replaces the earlier
four-GiB bitmap limit without introducing a fixed 128-GiB cap.

The bitmap indexes the sum of managed frames across physical ranges. Holes in
the physical address map consume no bits. Total managed RAM and highest physical
address are separate quantities.

## Bootstrap ownership

1. Decode and normalize Limine Usable ranges, excluding existing reserved types,
   and validate their CPU physical-address width.
2. Compute checked bitmap and summary lengths from that initial frame count.
3. Select one contiguous Usable span, rounded outward to complete metadata pages.
4. Reserve the span and normalize again. Require exact frame-count subtraction
   and no remaining overlap with allocatable ranges.
5. Validate retained HHDM bounds, initialize raw bytes, and form disjoint aligned
   mutable slices.
6. Transfer those exclusive buffers into the frame pool, then initialize the heap.

Sizing before the reservation intentionally leaves enough capacity after the
metadata pages themselves are excluded. The image owns the reserved pages
permanently; ordinary frame allocation and heap runs cannot obtain them. The
pool retains exclusive HHDM-backed buffer borrows for its lifetime. It validates
the complete plan and storage capacity before changing allocation state.
Initialization errors return the buffers unchanged.

Bootstrap initialization is claimed before carving or zeroing. A later call
must stop before touching metadata already borrowed by an active pool. A boot
failure does not create permission to reconstruct allocation state over live
frames.

The global heap cannot allocate this backing: its own allocation path already
needs an initialized frame pool. No Vec or per-frame allocation is needed during
frame-metadata bootstrap. Raw bytes are initialized before typed references are
created; no large temporary zero-filled array is placed on the entry stack.

## Address selection and lifetime

`allocate_in` restricts a frame request to a physical interval. It selects only
free frames already owned by the initialized pool. A range does not grant new
allocation authority, bridge a physical hole, or admit excluded metadata.
Invalid ranges and failed searches leave state unchanged.

General allocation need not proceed in ascending physical-address order.
Callers requiring low memory must request that interval explicitly. A high-frame
probe may change the allocation cursor without changing ownership rules.

High mappings retain the same frame tokens as low mappings. Scratch aliases
must be removed and their translations retired before returning or reusing data
and table frames. SMP removal and replacement still require acknowledgements
from every relevant online CPU; a missing acknowledgement retains backing.

## Size and remaining bounds

For a plan containing exactly 128 GiB in 4-KiB frames, the free-frame bitmap needs
524,288 u64 words (4 MiB). Its summary needs 8,192 words (64 KiB): 4.0625 MiB in
total before any page padding. Smaller inventories use proportionally smaller
storage. Metadata follows managed frame count rather than the highest address.

The boot map still has capacity for 256 raw entries and 288 normalized ranges.
One contiguous Usable span must fit metadata. CPU physical-address width,
canonical HHDM space, checked index arithmetic, and available RAM remain bounds.
Memory hotplug, fragmented metadata backing, NUMA policy, and migration of live
allocation metadata are separate work.

## Verification

Host tests model large frame inventories and sparse high addresses using metadata
buffers, without allocating a RAM-sized payload. They cover size arithmetic,
tail bits, failed initialization, returned buffer ownership, range selection,
exhaustion, release, and retry.

Boot checks require exact metadata reservation accounting and rejection of an
allocation request targeting the metadata span. Where Usable RAM exists above
4 GiB or 128 GiB, owned frames pass HHDM and scratch-alias read/write patterns,
zeroing, retirement, reuse, and restored frame counts. The guest memory-map
ranges determine which probes are required.

The [RAM evidence](../validation/2026-11-01-ram/evidence.json) identifies the final
host and QEMU configurations. Demand-backed QEMU exposes large guest RAM without
preallocating the entire guest backing on the host. The runner records and bounds
its own emulator's resident memory. This validates sparse allocation and address
handling. It does not sweep every RAM byte, measure hardware performance, or
establish physical large-memory reliability.
