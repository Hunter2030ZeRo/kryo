# Packed kernel heap

Date: 2026-11-04. Status: implemented with upstream Talc; validation linked below.

## Composition

Reuse Talc 5.1.1 for object allocation and its GlobalAllocSource for arena
acquisition/return. The existing page allocator remains the backing source.
Its private prefix retains the original OwnedRun token; no address reconstructs
allocation authority. No allocator fork or new slab algorithm is introduced.

Use explicit 4096-byte source blocks instead of Talc's one-MiB default. The
backing prefix can make a 4096-byte source allocation occupy two physical
pages. Object allocations share arenas, and empty secondary arenas return
their backing runs. The first arena holds Talc metadata for kernel lifetime.
It is neither an object leak nor permission to discard other live allocations.

The image owns an IRQ-excluded mutex around Talc and its client counters.
Disable local interrupts before acquiring the mutex. The lock order is object
heap then frame pool. Backing operations cannot allocate, enter VM/object policy
or wait for a remote CPU. NMI/emergency paths must not allocate. The heap is
SMP-safe but serialized; CPU-local caches require measured contention first.

## Failure and lifetime

Reject zero-sized or oversized layouts before entering the pinned source's
unchecked size rounding. Allocation returns null for unavailable backing and
metadata callers keep their existing fallible behavior. Place boot-test OOM
injection at the public allocator boundary so a free arena cannot bypass it.
Use GlobalAlloc's default realloc path through these checked alloc/dealloc
operations. Upstream in-place reallocation remains a possible later optimization.

Frame return follows the source's release callback and the original owner token.
The global allocator is never dropped. Its internal metadata is not a kernel
SOF handoff format; teardown/migration needs its own reviewed protocol.

## Accounting and evidence

Track backing pages separately from live client allocation count and requested
bytes. Physical checks balance free frames, retained table frames and global
heap backing once each. Since backing can shrink, compare sums rather than
subtracting unsigned growth. Logical checks independently require transient
allocation count/bytes to return to baseline, excluding only explicitly retained
kernel table-vector storage. Heap backing credit alone cannot prove no object
leak. Checkpoints require allocator quiescence.

Boot checks cover alignment, reuse, OOM, large layouts and 128 live 64-byte
objects. An AP mailbox probe holds varied allocations live simultaneously on
every participating CPU, verifies unique patterns, frees them and requires exact
heap-state restoration. It reuses the existing test mailbox mechanism rather
than creating a kernel thread framework.

The observed packing case needs eight additional backing pages, plus two pages
already retained for metadata. The old per-object algorithm requires 128 pages
by its checked page-size calculation. This comparison is a synthetic guest
memory-footprint result, not a latency or hardware-throughput measurement.

The host's floating nightly compiler changed from the previously validated
revision. The native ABI guard remains enabled; the new verification pin is
708c94ef093ecc42fb8851e17d8383d999727abc with Clang 23.1.4. Full native ABI runs
are required to accept this pin. See the
[validation record](../validation/2026-11-04-heap/README.md).

## Remaining work

This retains physically contiguous backing runs. Virtually contiguous backing,
complete per-process kernel-memory charging and parallel kernel policy remain
separate work. No libc allocator, TCB, scheduler or general kernel-SIMD change
is implied by this heap integration.
