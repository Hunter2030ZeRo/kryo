# Native user allocator

This static library reuses Talc 5.1.1 without modifying it. Its C bridge is
independent of libc initialization and TLS. The provider adapter translates null
allocation results into `ENOMEM`; this library does not set `errno`.

`__cryolite_allocate` requires a nonzero power-of-two alignment. A zero size
returns a distinct freeable minimum allocation when resources permit. The size
query returns the requested size, including zero, rather than unused capacity.
`__cryolite_reallocate(NULL, size)` uses the AMD64 malloc alignment of 16 bytes;
resizing a live allocation preserves its original alignment. Failure preserves
the original allocation and contents. Resizing a live allocation to zero frees
it and returns null. Deallocation of null is harmless. Other pointers must be
live original allocations from this bridge, as with ordinary C allocation APIs.

One private atomic/park/wake mutex protects Talc metadata. The fast path uses
only a compare-exchange; contended threads use the native compare-before-park
contract and bounded wake. The bridge does not allocate while waiting, disable
interrupts or depend on a runtime TCB. It is not async-signal-safe or a robust
mutex that recovers from a thread dying while holding it.

Backing comes from native reserve/commit/release. Commit failures return null
after releasing the unpublished interval. A failed rollback or release ends the
process so execution cannot continue after discarding a backing owner. The first
Talc metadata arena remains until process exit; empty secondary arenas return
their reservations. The global allocator is never dropped during execution.
This is separate from any future SOF teardown protocol.

The header prefix records the exact Talc layout and requested size. Layout
overflow is rejected before the pinned source's unchecked overhead arithmetic.
Reallocation first tries Talc's in-place operation, then allocates and copies
before releasing the original. No custom allocation algorithm or slab policy
is added here.

Host tests use tracked system backing and a condition-variable implementation
of the same compare-and-park contract. They exercise alignment, overflow,
exhaustion, allocation preservation, arena return and concurrent metadata access.
They do not execute native kernel calls. Real Ring 3 checks are required for
native VM and wait behavior.
