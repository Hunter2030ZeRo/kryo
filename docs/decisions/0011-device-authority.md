# Mediated device registers and interrupts

Date: 2026-11-01

## Boundary

The first device grant permits aligned 32-bit register operations through native
DeviceRead and DeviceWrite calls. Core validates the owning ProcessId, active
binding, complete access range, and window permissions. Handle rights are checked
separately. A register offset grants no physical-memory authority.

The kernel retains the supervisor UC/NX mapping. No device page is wrapped in a
RAM SharedPage or exposed through MemoryMap. Sub-page register windows prevent
adjacent control and routing registers from becoming accessible accidentally.
Direct user MMIO requires a distinct device-backing and cache-policy contract.

The image mints a grant for a known bootstrap process. There is no public device
discovery, binding, arbitrary physical map, or PCI configuration syscall yet.
General service discovery and driver selection remain user-space work.

## First hardware path

HPET comparator 1 provides a device path without DMA. Comparator 0 and the main
counter remain kernel clock mechanisms. Claim requires a second comparator with
64-bit and FSB capability, an initially disabled source, and retained HPET/APIC
mappings. Routing is fixed by the kernel to the BSP's D0 vector.

The driver receives READ|WRITE device authority and WAIT notification authority.
Its windows are the main counter's low word at 0xf0 (read-only), comparator-1
configuration at 0x120, and comparator value at 0x128. The HAL additionally checks
configuration values: fixed FSB/32-bit mode, optional interrupt enable, no periodic
or level mode. Global HPET control/status and FSB destination registers are not
granted. A counter sample and a comparison value are 32-bit wrapping quantities.

The IRQ handler records a coalesced pending event and acknowledges the APIC. A
user interrupt returns through ordinary context capture; kernel entry returns
without policy work. The BSP completes notification waits between execution
batches. No object/allocator/scheduler locks are taken in the handler.

Before deferred IRQ delivery, the BSP samples time and expires every due wait.
An IRQ first observed after that sample cannot satisfy an expired registration;
its pending notification remains available to a later wait. IRQ timestamps and
exact event-time ordering are not provided.

Notification delivery is coalescing. APIC EOI does not acknowledge a device's
register state, prove a deadline, or count exactly one event. The driver checks
its counter/comparator condition and filters early or redundant wakeups. The
HPET 32-bit one-shot wrap interrupt is one reason for this check.

## Lifetime and failure

One process owns the register authority; the initial IRQ notification has one
consumer thread. Process termination or consumer-thread death revokes the
binding. A surviving sibling cannot keep using an IRQ grant whose consumer died.
The device is not a wake source after its notification closes.

Active transitions to Quiescing before hardware cleanup. Cleanup disables the
source, verifies its control state, clears HPET status, drains queued D0 delivery,
and restores the saved disabled comparator/routing state. Only then does core
record Retired and image release retained notification references. Failed cleanup
keeps the claim, process/thread resources, mapping, and saved state for retry.
Drop does not perform hardware cleanup. Old handles observe revoked authority.

Arena locks do not span hardware access or interrupt-drain windows. The BSP keeps
the strong object and exclusive policy ownership while temporarily moving the
hardware owner out of its arena payload. Other CPUs execute only owned user jobs.

## Scope and later work

The bootstrap probe validates ordinary and timed IRQ waits, denied registers and
configuration, forced termination with an armed source, consumer-thread exit,
pending-IRQ cleanup, and failed retirement retry. It runs as a separate Ring 3
ELF; bootstrap image code supplies its grants. This is not a general device manager.

DMA remains unavailable. No bus mastering, device-address loan, pin API, PCI/INTx
routing, or IOMMU domain is introduced. Those require independent device authority,
page pin accounting, quiescence, and translation-retirement proofs. The HPET D0
drain is not a generic PCI/MSI/DMA fence.

## References

- [Intel HPET specification](https://www.intel.com/content/dam/www/public/us/en/documents/technical-specifications/software-developers-hpet-spec-1-0a.pdf), timer configuration, comparator, FSB route, and 32-bit mode.
- [QEMU HPET implementation](https://github.com/qemu/qemu/blob/master/hw/timer/hpet.c), optional `hpet.msi` capability used by the guest fixtures.
