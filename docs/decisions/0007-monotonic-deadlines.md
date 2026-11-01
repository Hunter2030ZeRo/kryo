# 0007: Monotonic deadlines on the BSP

Date: 2026-11-01

## Decision

Use the ACPI-described HPET main counter as the first monotonic clock. Require
a 64-bit counter and a valid declared period. Do not infer elapsed time from
interrupt counts, CPU loops, or an assumed TSC frequency. The clock begins at
the counter value observed during initialization; its nanoseconds are unrelated
to UTC and may have equal consecutive readings.

Firmware discovery validates RSDP and RSDT/XSDT checksums and bounded table
lengths. Table entries are physical addresses. Limine base revision 6 supplies
a virtual RSDP address. Read referenced tables through a temporary read-only
mapping after checking retained firmware-memory provenance. Map HPET separately
as supervisor-only, uncached, writable, and NX.

Calibrate the local APIC countdown against a bounded HPET interval. User entry
arms a one-shot event for the earlier of its two-millisecond quantum and the
next pending deadline. Idle arms the earliest pending deadline and uses the
existing `sti; hlt; cli` sequence. Convert durations upward to countdown ticks;
clamp long intervals to the hardware range and recheck the clock on an early
chunk wake. Calibration and polling failures are explicit errors.

The HPET counter continues across user execution and idle. Timer IRQ handlers
only record and acknowledge delivery. Deferred code samples the clock and
expires waits after IRQ/context return. A stale or early IRQ is not evidence
that a deadline has elapsed. Interrupts remain disabled during kernel work,
so this is not a bound on kernel scheduling latency.

## Wait ownership

Store an optional absolute nanosecond deadline in the existing generational
wait slot. The fixed-capacity table needs no second allocation or timer ID.
Select the earliest pending deadline with a bounded linear scan; equal times
use slot order. Completed waits immediately cease contributing a deadline.
The wait table and its clock domain persist while object queues retain IDs.

Endpoint and notification timeout paths detach registrations before completing
the wait. They share cancellation's queue and reply-token retirement rules.
Detach all waits due at the sampled time before allowing endpoint progress;
freeing a reply slot must not deliver another already-expired request.
The first serialized completion wins. Parking still hides a completion until
CPU execution handoff finishes. Commit accepted capability grants before any
sender cleanup, including timeout. Timeout after acceptance reports an uncertain
outcome; it does not revoke delivered capabilities, undo work, or end a future
buffer loan.

Sleep uses the same wait table with a separate bounded record identifying its
owner as the timer. Cancellation and task death remove that record. Neither
deadline scans nor hardware IRQs publish runnable tasks directly: saved results
are installed before scheduler wakeup, outside object access.

## Experimental calls

`ClockRead` returns monotonic nanoseconds. `SleepUntil` uses an absolute deadline
and succeeds immediately if it has passed. The six timed IPC variants retain
the existing call arguments and take an absolute deadline in RDX. A deadline
already reached at admission returns TimedOut before object publication, even
if a rendezvous could otherwise succeed. Use untimed calls for indefinite waits.

Timeout returns RAX=-10 and delivery state in RDX: zero before acceptance, one
after acceptance. A late reply cannot revive a completed wait. Absolute deadlines
avoid extending an operation when a caller retries or is preempted. These calls
remain version 0 and are connected by boot-test wiring.

## Limits

The initial clock requires ACPI and a supported HPET. There is no PIT, PM timer,
or invariant-TSC fallback, 32-bit counter extension, suspend/resume clock policy,
SMP clock synchronization, or calibrated real-hardware latency/power result.
The BSP owns the clock, APIC, mappings, and configuration until explicit shutdown.
HPET comparator IRQs and legacy replacement stay disabled during ownership.
Drain the APIC before releasing its mapping; stop clock consumers and restore
HPET configuration before releasing the clock mapping.

The implementation is a timer mechanism for the existing bounded runtime.
Normal-boot init startup, fallible process creation, termination notification,
and user-space supervisor policy are the next integration work.

## References

- [Intel HPET specification 1.0a](https://www.intel.com/content/dam/www/public/us/en/documents/technical-specifications/software-developers-hpet-spec-1-0a.pdf)
- [ACPI system description tables](https://uefi.org/specs/ACPI/6.6/05_ACPI_Software_Programming_Model.html)
