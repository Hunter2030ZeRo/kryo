# PCI and DMA storage validation

Date: 2026-11-02.

All 16 QEMU MTTCG configurations pass. Each configuration below ran debug/release
with both boot-tests and normal supervision. `evidence.json` retains commands,
checks, fixture hashes and sampled resident memory. Raw guest/stdout/stderr logs
are alongside it. `source-manifest.json` hashes 186 source, configuration and
runner inputs, unchanged throughout the matrix. Generated builds and prose are
excluded.

| CPU model / count | Guest RAM | PCI block fixture | Runs | Peak RSS MiB |
| --- | --- | --- | --- | --- |
| default / 1 | 512 MiB | absent | 4 | 123.69 |
| default / 1 | 512 MiB | read-only, behind bridge | 4 | 125.53 |
| max / 4 | 512 MiB | read-only, behind bridge | 4 | 134.08 |
| max / 2 | 129 GiB | read-only, behind bridge | 4 | 151.49 |

## Host checks

`host-tests.log` records 357 passing Rust tests: core 276, HAL 64, interfaces 2,
libsys 4 and services 11. Six Python evidence-parser tests also pass. New cases
cover PCI identity, absent functions, multifunction headers, nonzero buses, raw
64-bit BAR pairs, configuration errors, DMA address bounds, allocation rollback,
retirement retry and ownership retention during abandonment or panic.

`checks.log` records Rustfmt, normal/all-feature kernel Clippy and scoped host
core/HAL Clippy with `-D warnings`. No lint exclusion was needed. The existing
metadata release now uses `mem::take` followed by drop, preserving capacity
deallocation instead of following the current nightly's `clear` suggestion.
The recorded compiler is Rust 1.101.0-nightly, 2026-11-01.

Independent review found no blocker and separately passed 61 focused tests. It
checked the completed debug PCI run before the final matrix finished; the matrix
results above were subsequently collected by the main validation runner.

## Guest checks

The read-only raw fixture appears as modern VirtIO block `1af4:1042` on bus 1
behind a configured PCIe root port. The guest scans segment zero and reports
unchanged command registers. The existing multifunction chipset also appears in
the inventory. The runner checks unique BDFs, exact inventory counts, a nonzero
storage bus and the fixture hash after stopping its own emulator.

The scanner does not write device configuration or probe BAR sizes. The observed
firmware command value `0x0107` already includes bus-master enable; unchanged
command bits do not prove a disabled or quiescent device. Kryo submits no queue
or buffer address in this phase.

The unpublished-buffer check allocates three real contiguous frames under a
32-bit physical/device address bound, initializes each full page RW+NX, verifies
every byte is zero, dirties the bytes, then explicitly retires ownership. It runs
twice. Frame counts restore exactly except measured retained alias-table storage.
The backend describes identity addressing but creates no hardware translation
and never exposes the buffer to a device. Failed quiesce/unmap and translated
device addresses are host-model evidence only.

All prior guest memory, SMP/TLB retirement, IPC, capability, budget, thread/TLS/
SIMD and service-replacement checks still pass. Normal boots retain the active
supervisor; boot-tests retire their test resources and end with ABC.

## Boundaries

There is no block request, disk read/write/flush, hardware DMA backend, user DMA
API, pin accounting, IOMMU domain or general device-manager service. The raw
fixture has no filesystem and is not a BetAFS image. BetAFS mounting/file I/O is
not implemented. Its independent engine still requires a native runtime port;
the [adapter contract](../../../userspace/filesystems/betafs/README.md) preserves
4096-byte blocks and actual durable flush.

The runner enforces a 90-second boot limit and 2048-MiB sampled RSS limit and
terminates only its owned emulator after the markers. This does not verify guest
shutdown. 129-GiB guests are demand-backed and sparsely touched, not a full RAM
sweep. No physical hardware, power-loss durability, latency or energy claim is
made.

## Reproduce

Run `python3 tools/boot_smoke.py --all`, then add `--pci-probe` for a disposable
read-only block fixture. Add `--normal` for persistent supervision. The SMP
selection is `--cpu max --cpus 4`; the sparse large-RAM selection is
`--cpu max --cpus 2 --memory 129G`. Run all four selections in both modes for the
16-run matrix. Fixture output uses a separate `-pci` directory and never replaces
`build/debug.log`.

Host commands are recorded verbatim in `host-tests.log` and `checks.log`. Run
`python3 -m unittest discover -s tools` for the runner evidence checks.
