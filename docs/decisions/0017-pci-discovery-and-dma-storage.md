# PCI discovery and DMA storage ownership

Date: 2026-11-02. Status: implemented foundation; storage I/O pending.

[ADR 0018](0018-mediated-virtio-block-transport.md) extends this foundation with
validated private queues and actual Ring 3 block commands.

## Discovery

The PC bootstrap scans conventional PCI segment zero through CF8/CFC. Each
selector/data transaction excludes local IRQs and holds the HAL configuration
lock; it restores the previous selector. Bus numbers span 0..255, with function
zero and the additional functions advertised by a multifunction header.
Inventory storage grows fallibly. Hotplug, other segments and ECAM are outside
this path.

Snapshots retain BDF, identity, class, command and raw BAR registers. They grant
no device access. The scan does not size BARs, change command bits, configure
bridges or enable queues. Firmware-assigned command bits, including bus-master
enable, are left unchanged. BAR size validation and device initialization must
precede any later grant. Read-only discovery is not device quiescence.

HAL owns configuration transactions; image retains the boot inventory. Driver
selection, service startup and storage policy remain user-space responsibilities.
No PCI configuration syscall or general device-manager service is added here.

## DMA storage

Core `DmaBuffer` retains one physically contiguous `OwnedRun`, a backend and its
mapping. Limits bound page count, eligible physical span and the inclusive last
device address. Physical and device addresses may differ. A physical constraint
only restricts existing frame-pool authority; it cannot admit reserved RAM.

An unsafe backend initializes the entire run with a retained RW+NX CPU alias,
then maps it without starting device access. Initialization and mapping errors
must leave unpublished backing safe to return. The complete mapped device range
is checked for overflow and address-mask coverage.

Publication records possible access before hardware submission. Retirement
blocks publication, quiesces outstanding access, removes translations and
finishes their invalidation, then releases RAM. A failed step returns the buffer
with every remaining owner. Retry resumes at that step. Constructor rollback
also returns a retained owner when cleanup fails. Drop does no hardware work;
abandoning an owner retains backing, backend and translation rather than recycling
potentially active memory.

This is a bootstrap allocator, not a user DMA ABI. It has per-buffer page bounds,
but no process DMA quota, shared-memory pin operation or hardware backend yet.
The private image backend never submits the range to a device and installs no
IOMMU mapping. Its no-op quiesce/unmap are valid only for that unpublished check.
Host models test failures and a translated device address. Guest checks allocate,
zero, dirty and retire real frames; they do not demonstrate DMA completion.

## Storage boundary

The selected filesystem is BetAFS. Its engine stays in the independent project;
Kryo contains only the native adapter. The current BetAFS block contract requires
4096-byte blocks, read-your-writes and a durable sync barrier. A future 512-byte
sector transport must check the conversion to eight sectors and complete the
entire block. A successful flush cannot be synthesized from submission or IRQ
completion. Failed flush leaves commit state uncertain and requires reopening
the BetAFS store.

The existing BetAFS engine keeps its normal `std` dependency. Its native execution
requires the Cryolite Rust target and standard-library platform support described
in [ADR 0019](0019-native-runtime-and-standard-libraries.md). Do not duplicate its
disk parser or substitute another filesystem for the storage demonstration.

## Next gate

Before a Ring 3 VirtIO driver can use DMA, implement a validated submission path
or an IOMMU-backed DMA domain. A mutable queue and raw PCI register authority
would otherwise let a driver address arbitrary RAM. Allocating a small buffer
alone does not contain it. Device reset, IRQ retirement and translation teardown
must retain owners on failure. The next block transport must provide real read,
write and flush before the BetAFS adapter can mount or serve files.

## References and evidence

- [Linux mechanism-1 transactions](https://github.com/torvalds/linux/blob/master/arch/x86/pci/direct.c)
- [VirtIO 1.2](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html)
- [Validation](../validation/2026-11-02-storage/README.md)
- [BetAFS adapter boundary](../../userspace/filesystems/betafs/README.md)
