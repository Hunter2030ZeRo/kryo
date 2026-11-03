# Private VM and atomic waits

Date: 2026-11-03. Status: proposal for maintainer review; no new calls implemented.

## Private memory proposal

Give the native runtime page-backed anonymous private memory. User allocators
manage arenas, small allocations and caches. Kernel mechanisms validate address
ranges/protection, supply zeroed pages, retain backing and retire mappings.
Virtual contiguity must not require physically contiguous RAM.

Do not use the bootstrap MemoryCreate limit of eight shared pages as the general
heap contract. Private, shared and device-accessible memory have distinct
authority and lifetimes. File-backed mappings additionally need pager/VFS policy.

Proposed first scope: non-executable anonymous RW mappings, page-aligned ranges,
explicit failure, no replacement of an existing mapping, and bounded resource
accounts. Reserve metadata before publication; failed preparation retains owners
until cleanup completes. Unmap/reuse includes acknowledged TLB retirement and
all retained uses; accepted unmap is not permission to free active storage.
Define partial unmap and accounting before adding them to a published ABI.

The runtime can obtain larger arenas and suballocate without a kernel call per
malloc. Measure physical-byte amplification, fragmentation, growth/release
latency and idle costs; no allocator performance is inferred from libc choice.

Native thread creation must eventually accept runtime-owned stack/TLS resources
through a checked unpublished transaction. Preserve compatibility with earlier
boot conventions while removing redundant scratch allocations from the general
runtime. A logical exit flag cannot authorize premature stack/TLS reclamation.

## Atomic wait proposal

Keep uncontended locking in user-space atomics. On contention, enter a native
compare-before-park operation with an absolute monotonic deadline. Comparison
and waiter registration must be serialized with wake so a store followed by
wake cannot fall between them and lose the event. Reuse the existing Parking
and single-completion rules for wake, timeout, cancellation and peer death.

Initial private waits can be keyed by address-space and retained mapping
identity. Reusing a virtual address must not inherit an old waiter. Shared waits
need backing-object identity and offset, not an integer VA or physical address
alone. Resolve that model before exposing process-shared pthread operations.
Unsupported modes return errors.

Reserve fallible wait metadata before taking short queue locks. Validate the
word's alignment/range and retain readable backing through the compare/register
transaction. Raw user memory must not be borrowed as trusted Rust data. Unmap,
object closure and final cancellation need defined queue removal and lifetime
rules before storage can be reused.

Measure wake-to-run latency, contention scaling, per-wait allocation and idle
wakeups on the actual scheduling path. BSP-serialized policy and linear wait
scans remain existing limits, not the final concurrent implementation.

## Review before implementation

Confirm anonymous VM ownership/accounting, initial protection/partial-unmap
scope, private/shared wait keys, and completion/retirement semantics with the
maintainer. Syscall numbers and public records remain experimental. Neither
mlibc internals nor Linux syscall encoding selects Cryolite's kernel ABI.
