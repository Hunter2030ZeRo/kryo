# 0008: Process startup and user-space supervision

Date: 2026-11-01

## Scope

The next integration step is normal-boot init, fallible creation of one-thread
processes, retained exit observation, and a restart chosen in user space. It
does not depend on a running filesystem, compatibility layer, or device driver.

## Ownership

A process owns its address space and handle table. Its initial thread owns its
saved CPU state, guarded kernel entry stack, TLS base, and wait registrations.
Keep these responsibilities separate even while one scheduling resource bundle
contains them. This stage supports one thread per process. Sharing process
resources among several threads requires a later registry and synchronization
contract; adding another thread is not equivalent to creating a process.

Process-control authority refers to one generation of one process. A retained
control object records the terminal result independently of the live VM and
thread resources. Reusing a scheduler slot cannot make an old control handle
refer to the replacement. Closing a control handle does not terminate the
process; stopping a process requires CONTROL.

## Creation transaction

Boot modules supply immutable static native ELF images. Execution requires an
image capability with EXECUTE authority. A filename or integer address does
not grant execution or memory authority. The initial boot manifest selects
init and explicitly supplies its initial image and debug-output authority.

Creation validates the ELF plan, capacities, and requested startup grants before
publication. It privately prepares the user root, executable pages, zeroed data,
guarded user stack, initial CPU state, kernel entry stack, and destination handle
table. Startup grants reuse bounded reduced-rights copy delegation. The parent's
original handles remain valid. No unrelated parent handle is inherited.

Reserve or verify all bounded publication capacity before making the child
runnable. The single-BSP transaction runs with kernel IRQs off. Failure returns
an ordinary error and explicitly rolls back unpublished mappings and stacks.
If retirement cannot finish, retain the allocation owners for cleanup retry;
failure must not free memory that may still be accessed. Boot failures needed
to create init may halt with diagnostics.

## Termination

User exit, fatal user fault, and authorized stop produce distinct terminal
reasons. Stop CPU execution and detach IPC, notification, sleep, and observation
waits before reaping execution resources. Preserve accepted transfer grants
before sender cleanup. VM/stack cleanup may be retried without publishing a
second terminal result or replacing the first reason.

Exit observation is level-triggered. A waiter registered before termination
and an observer arriving after resource reclamation both receive the same
retained result. A timeout or cancellation detaches only that observation;
it does not stop the target. Wait completion follows the existing Parking
handoff and saved-result-before-wakeup rules.

Exit observation reports that execution ended. It does not promise device
quiescence, DMA revocation, completion of every cleanup, or rollback of accepted
requests. Reclamation and external references retain their existing ownership
rules.

## Supervisor policy

Init runs as a separate native ELF through the ordinary execution and syscall
paths. Its first bounded supervision scenario starts a worker, waits for its
terminal result, and creates a replacement from retained image authority.
Restart count, delay, and failure policy are user-space choices. The kernel
does not restart tasks automatically or select drivers and services.

A service instance generation is separate from process and endpoint identity.
Keep old control handles and endpoint references bound to the old instance.
Future service discovery must publish a new generation and reject stale
requests; this first restart scenario establishes process lifetime, not service
state restoration or transparent reconnection.

## Implementation order

1. Retained terminal state and bounded wait/cancel/timeout rules, with host tests.
2. Common image runtime and fallible ELF/stack creation, shared by normal boot
   and guest checks.
3. Capability-scoped spawn and process observation calls, with separate native
   init/worker builds and explicit initial authority.
4. Normal-boot execution and restart/lifetime/failure checks in QEMU.

Then add a general startup record and runtime conventions, service discovery,
termination subscriptions, separate supervisor/device-manager services, and
resource quotas. IRQ/MMIO/DMA authority and device isolation follow those
contracts before general driver or filesystem integration.

## Evidence

[Implementation status](../status.md) records host and guest coverage. Normal
boots and regression boots run separately on default and AVX-capable QEMU CPUs
in debug and release. Regression checks include failed publication capacity,
explicit rollback, stale process controls, and final allocation reclamation.
The retained debug stack failure is documented with the phase-lifetime fix.

This stage does not publish a stable ABI, enable SMP, share one process among
multiple threads, or implement service discovery, device recovery, and DMA
isolation. Normal init remains parked after its bounded supervision scenario;
its long deadline uses the timer's bounded countdown chunks rather than a
periodic user-space polling loop.
