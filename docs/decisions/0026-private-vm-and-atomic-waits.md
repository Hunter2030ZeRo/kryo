# Private VM and atomic waits

Date: 2026-11-03. Status: accepted. Explicit private VM is implemented in the
experimental version-0 ABI; atomic park/wake remains unimplemented.

## Private memory

Use separate reserve, commit, decommit and release operations. The process owns
each reservation. Its trusted resource account supplies authority and limits;
a returned address is not authority over arbitrary physical memory. User
allocators manage arenas, small allocations and caches.

| Operation | Contract |
| --- | --- |
| Reserve | Choose a vacant aligned range, retain metadata and charge reserved virtual pages; allocate no data frames or accessible user pages |
| Commit | Supply actual zeroed private RW/NX RAM for an uncommitted subrange before returning success |
| Decommit | Retire committed backing in a subrange and refund its private-page charge; retain the virtual reservation |
| Release | Retire all committed backing, then remove the reservation and refund its virtual-page charge |

Ranges are nonempty multiples of 4096 bytes. Reserve alignment is a power of two
at least 4096. The kernel chooses first-fit placement in
`[0x1_0000_0000, 0x2000_0000_0000)`, avoiding existing mappings and reservations.
Commit/decommit ranges must be aligned and lie entirely within one reservation.
Commit rejects any already committed page. Decommit accepts uncommitted holes;
release accepts holes or an empty reservation and requires its exact base.
Ordinary shared-memory map/unmap calls cannot bypass reservation ownership.

Commit acquires all requested data frames before publishing the first user PTE.
Frames are individually owned; virtual contiguity does not require physical
contiguity. Zero initialization finishes before successful return. There is no
overcommit promise or allocation on first access. Access to an uncommitted page
remains a user fault. Protection changes, executable/JIT memory, file-backed
paging, resizing and partial reservation release remain outside this API.

## Accounting and retirement

Shared accounts separately charge reserved virtual pages and explicitly
committed private pages. Existing ELF, managed stack and legacy TLS backing are
not included in the new private-page counter. Mapping/page-table limits remain
per process and include the root table. These limits do not account for every
kernel metadata byte or CPU cycle.

Decommit/release return data-frame credit and, for release, reserved-page credit.
AddressSpace retains intermediate page-table frames until full process-space
close. Repeated sparse mappings can therefore consume the per-process table
limit even after data and reservations have been released. Table pruning and
compaction are not implemented in this milestone.

Quota or allocation failure returns an ordinary error. Failed commit restores
the preceding usable state. If rollback or retirement cannot complete, retain
the backing, reservation and charges and quarantine the whole process; do not
resume with a partially published failed commit. Decommit/release return frame
credit only after user access, required TLB acknowledgements and backing-owner
cleanup finish. Explicit teardown retries retained resources. `Drop` must not
free backing still reachable through a live translation.

The image serializes these calls on the BSP after every CPU has returned from
the current user execution batch. This preserves the existing SMP execution
and retirement contract; it does not introduce parallel kernel VM policy.

MemoryCreate's eight-page shared-object limit is independent of this API.
Private, shared and device-accessible memory retain distinct lifetimes.
The runtime can suballocate larger private arenas without one kernel call per
malloc. This kernel interface does not implement an allocator, libc or Rust std.
Measure fragmentation, physical-byte amplification, growth/release latency and
idle costs before claiming performance improvements.

Mapping lookup uses a sorted vector with binary search; range retirement batches
metadata removal. Inserting an interior committed range still shifts the suffix
once per page, with worst-case work proportional to pages times suffix length.
Fragmented-heap insertion and reservation scans are not yet optimized. These
mechanisms establish ownership and failure behavior, not measured hardware
performance or allocator efficiency.

Native thread creation must eventually accept runtime-owned stack/TLS resources
through a checked unpublished transaction. Preserve compatibility with earlier
boot conventions while removing redundant scratch allocations from the general
runtime. A logical exit flag cannot authorize premature stack/TLS reclamation.

## Atomic wait direction

Keep uncontended locking in user-space atomics. On contention, enter a native
compare-before-park operation with an absolute monotonic deadline. Comparison
and waiter registration must be serialized with wake so a store followed by
wake cannot fall between them and lose the event. Reuse the existing Parking
and single-completion rules for wake, timeout, cancellation and peer death.

Start with process-private waits. Their keys must include address-space and
retained mapping identity so address reuse cannot inherit an old waiter.
Shared waits need backing-object identity and offset, not an integer VA or
physical address alone. Resolve shared lifetime rules before exposing
process-shared pthread operations. Unsupported modes return errors.

Reserve fallible wait metadata before taking short queue locks. Validate the
word's alignment/range and retain readable backing through the compare/register
transaction. Raw user memory must not be borrowed as trusted Rust data. Unmap,
object closure and final cancellation need defined queue removal and lifetime
rules before storage can be reused.

Measure wake-to-run latency, contention scaling, per-wait allocation and idle
wakeups on the actual scheduling path. BSP-serialized policy and linear wait
scans remain existing limits, not the final concurrent implementation.

There are no native park/wake calls yet. Existing notifications are not an
atomic compare-and-park substitute. Word representation, fault-safe access,
mapping retention and queue removal on decommit/release must be fixed before
implementation. Syscall numbers and public records remain experimental.
Neither mlibc internals nor Linux syscall encoding selects Cryolite's ABI.
