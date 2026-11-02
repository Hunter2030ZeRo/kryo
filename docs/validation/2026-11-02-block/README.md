# Mediated block transport validation

Date: 2026-11-02.

All 24 final QEMU MTTCG runs pass. Each selection below ran debug/release in
boot-tests and normal supervision. `evidence.json` records commands, checks,
fixture hashes and sampled resident memory. The 72 raw guest/stdout/stderr logs
are retained alongside it. `source-manifest.json` hashes 194 source/configuration/
runner inputs, unchanged throughout the matrix; prose and generated builds are
excluded. Peak sampled QEMU RSS was below 154 MiB with the 2048-MiB guard.

| CPU model / count | Guest RAM | Device selection | Runs |
| --- | --- | --- | --- |
| default / 1 | 512 MiB | no extra device; block probe disabled | 4 |
| default / 1 | 512 MiB | read-only PCI discovery fixture; block probe disabled | 4 |
| default / 1 | 512 MiB | writable signed block fixture | 4 |
| default / 1 | 512 MiB | read-only signed block fixture | 4 |
| max / 4 | 512 MiB | writable signed block fixture | 4 |
| max / 2 | 129 GiB | writable signed block fixture | 4 |

## Host and build checks

`host-tests.log` records 387 passing Rust tests: core 296, HAL 73, interfaces 3,
libsys 4 and services 11. Nine Python evidence-parser tests also pass. New tests
cover capability-list boundaries, register phase gates, offset-plan validation,
private hardware links, malformed completions, stale tickets, poisoning/reset,
65,537 queue completion cycles, and temporary buffer initialization rollback.

`checks.log` records Rustfmt for both workspaces, normal/all-feature kernel
Clippy, scoped changed core/HAL tests and production userspace workspace Clippy,
all with warnings denied and no lint suppression. `packaging.json` verifies all
22 debug/release boot images preserve ELF entry/load data and fit the unchanged
16-page load bound. The Ring 3 block image uses the same native target and does
not depend on privileged core/HAL crates.

Independent review found and rechecked two corrected integration issues:
initialization now acknowledges DRIVER before feature queries and accepts
FEATURES_OK before queue discovery; explicit reset prepares renegotiation and
the driver tests actual restart. Its final scoped review found no blocker and
independently passed HAL 73, queue 17 and DMA 17 tests. It inspected the completed
read-only QEMU run; the full matrix was subsequently collected by the main runner.

## Actual block operations

BLOCK_PROBE=1 adds an explicit enable module. Ordinary images omit it. The
runner creates an owned fresh 8-MiB file and attaches a modern VirtIO block device
behind a PCIe root port. The native driver reads block 0 and checks its signature
before any write.

For each writable selection, mode 0 writes exactly block 1 (sectors 8..15),
submits VIRTIO_BLK_T_FLUSH and requires successful status/used length, then reads
back all 4096 pattern bytes. It resets, negotiates again, reads another block and
rejects the old ticket. After stopping its own emulator, the runner hashes the
whole file: only [4096,8192) may contain the expected pattern
`((offset * 37 + 11) & 255)`. Every other byte, including the signature, must
match the original. This is guest flush completion plus host persistence/readback
evidence; it is not a physical power-loss durability test.

Read-only selections require device RO negotiation, an actual write status of
IOERR with the expected used length, and an unchanged whole-file hash. They do
not synthesize a successful write or flush. PCI discovery-only runs also retain
an unchanged fixture hash and never enable the block driver.

The Ring 3 driver mutates its shared descriptor after accepted submission. The
private accepted plan still returns correct bytes to its original destination.
Out-of-bounds tables, overflowed data ranges, unsupported flags, a wrong-kind
memory handle, busy submission, read-only control authority and stale tickets
are rejected. A separate foreign process receives the same Device object with
READ/WRITE/CONTROL and must be denied all five transport calls by ProcessId.

## Lifetime and recovery

Mode 1 submits a read and exits without polling or closing its memory. The
coordinator retains the private DMA allocation, pending original memory and
device claim until reset completes. This proves retirement of accepted work;
the device may already have completed by the time the process exits.

Boot tests inject one declined cleanup attempt before hardware retirement and
require unchanged frame counts plus retained queue/pending memory. This is an
injected software failure, not a measured failing hardware reset. Retry then
finishes. Both modes release their processes, objects and DMA storage. Frame
counts restore except measured retained kernel alias-table storage. Two driver
instances reclaim the same function in turn.

The initial foreign-caller fixture selected an overlapping entry-stack slot.
The existing mapping check rejected it before publication; the retained
`prior-readonly.debug.log` records AlreadyMapped. The fixture now obtains both
cells through the owned stack allocator. No stack/image bound, watchdog,
scheduling quantum or expected device result was enlarged to pass validation.

## Limits and reproduction

The driver is an opt-in validation image using native calls. There is no
persistent block IPC service, VFS, BetAFS mount or native standard-library port.
The disk fixture has no filesystem. BetAFS keeps its normal Ring 3 std dependency
through the future Cryolite Rust target/std platform support.

The transport uses configuration-window I/O and completion polling, one request,
three descriptors and an 8-KiB private payload bound. No storage IRQ, IOMMU or
isolation against malicious hardware is claimed. Existing memory, SMP/TLB,
IPC/budget, thread/TLS/SIMD and supervision checks still pass. The 129-GiB guest
is demand-backed and sparsely touched, not a full RAM sweep. The runner enforces
its 90-second boot timeout and stops only its owned emulator after markers;
guest shutdown, hardware latency and energy are not verified.

Run `python3 tools/boot_smoke.py --all` for default boots and add `--normal` for
persistent supervision. `--pci-probe` selects discovery only. `--block-probe`
selects the signed writable fixture, with `--block-readonly` for device-rejected
writes. Add `--cpu max --cpus 4`, or `--cpu max --cpus 2 --memory 129G` for the
large sparse selection. Each mode/profile retains separate output and preserves
`build/debug.log`. Host and lint commands are recorded in their logs.
