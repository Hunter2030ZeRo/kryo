# Private VM and atomic waits

Date: 2026-11-03. Status: accepted. Explicit private VM and process-private
atomic park/wake are implemented in the experimental version-0 ABI.

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

Native thread creation borrows runtime-owned stack/TLS resources through the
checked transaction in [ADR 0027](0027-runtime-owned-thread-memory.md). Active
execution leases also protect optional unmapped guards. Admitted private-VM
commit, decommit and release conflicts return Busy before invalidation or
retiring state; they do not quarantine the process. Legacy MemoryMap/Unmap keep
their InvalidArgument exclusion for reserved private-VM arenas before reaching
the core lease check. Actual completion releases the lease
before native join, while user mappings and their quota remain owned by the
runtime. A logical exit flag cannot authorize premature reclamation.

## Atomic private waits

Keep uncontended locking in user-space atomics. Park compares one four-byte
aligned native `u32` in an owned private RW mapping. This includes ELF data/BSS,
managed stack/TLS and explicitly committed private VM. Shared, read-only,
executable, uncommitted and unpublished backing is rejected. The kernel copies
four bytes through validated owned RAM while all user CPU leases are quiescent;
it creates no Rust reference into user storage.

| Operation | Contract |
| --- | --- |
| Park | Return ValueChanged if the word differs from expected; otherwise register a wait |
| ParkUntil | The same comparison, with an absolute monotonic nanosecond deadline; an equal word with an already due deadline returns TimedOut |
| Wake | Detach up to the requested number of matching waiters in admission order and return that count; zero detaches none |

Wake neither writes the word nor buffers an event for a future waiter. The
runtime publishes its predicate with a Release store before wake and rechecks
it with an Acquire load after park. A successful wake does not acquire the
runtime's lock. Callers loop around the predicate and handle timeout or
cancellation explicitly. These operations do not replace coalescing
notifications or expose Linux syscall compatibility.

Comparison and queue admission share BSP ownership with wake and mapping
retirement after every CPU returns from its execution batch. A store followed
by wake cannot fall between comparison and registration. Each incoming native
call samples due deadlines before progressing requests; a deadline which
becomes due during an earlier serialized call is considered at the next call's
admission. This is sampled timeout ordering, not a scheduling-latency guarantee.
Value mismatch returns without registering a wait, even when the supplied
deadline is already due.

Keys combine unique address-space identity, monotonic mapping identity and the
exact word address. Sorting metadata does not change a key; unmap/remap or
address-space replacement does. Keys grant no authority and do not pin backing
while asleep. The kernel detaches affected registrations before admitted
decommit/release, ordinary private-page unmap, managed stack/TLS cleanup or final
process-space retirement can free storage. Retained failed thread-creation
pages can be observed by their process; cleanup retries invalidate them too.
Cancellation completes detached
waits with Cancelled. Failed validation or VM metadata preparation leaves the
registration intact. Cancellation does not itself promise memory retirement.

Native thread exit releases its execution lease but leaves runtime-owned
storage mapped, so it does not by itself cancel waits on that storage. Reusing
an atomic or TCB in place requires a runtime protocol to drain old waiters;
mapping identity changes only when the backing mapping is replaced.

Only actual word waits reserve word-queue capacity, after comparison/deadline
checks and before charging or publishing a wait. The persistent wait table
owns the Waits charge through pending and Ready states. Wake, timeout,
cancellation and task death detach their queue records before completion or
abandonment; one completion wins. Image code writes saved return values before
publishing a runnable thread. Queue completion and invalidation allocate no
metadata.

Queues and deadline selection still use linear scans under serialized BSP
policy. Measure contention, wake-to-run latency, metadata cost and idle wakeups
before claiming scalability or efficiency. Shared waits, requeue, priority
inheritance and robust-mutex recovery remain unimplemented. Shared waits need
backing-object identity and offset, with separate lifetime rules. A user mutex
implementation, full pthread/TCB integration and libc/std ports remain separate
work. Syscall numbers and public records remain experimental; neither mlibc
internals nor Linux syscall encoding selects Cryolite's ABI.
