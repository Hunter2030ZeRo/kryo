# Process-private atomic waits

Date: 2026-11-03. Contract: [ADR 0026](../../decisions/0026-private-vm-and-atomic-waits.md).

Park compares a four-byte-aligned native u32 in a published, owned private RW
page. Mismatch returns ValueChanged without a wait charge. A matching word
registers with the existing wait table before the thread can sleep. Wake removes
up to the requested number of matching FIFO registrations, then completes them.
Wake does not change the word or retain an event for later callers.

ParkUntil uses an absolute monotonic deadline. Each native admission samples
expiry before processing wake, cancellation or mapping operations, including
after an earlier long call in the same CPU batch. Ready waits retain their
charge until completion consumption. Users must publish with their own atomics
and recheck state after every return.

## Validation

Six QEMU TCG runs passed with 512 MiB guest RAM:

| Profile | CPU model / processors | Mode | Optional modules |
| --- | --- | --- | --- |
| debug | Haswell-noTSX / 1 | boot-tests | ABI probe |
| release | Haswell-noTSX / 1 | boot-tests | ABI probe |
| debug | max / 4 | normal | ABI probe, writable block fixture |
| release | max / 4 | normal | ABI probe, read-only block fixture |
| debug | max / 4 | boot-tests | legacy modules |
| release | max / 4 | normal | legacy modules |

The four ABI runs execute real Ring 3 Park/ParkUntil/Wake calls. They check
uncharged mismatch, no buffered wake, invalid/RO addresses, two queued FIFO
waiters with wake limits zero/one/all, timed completion, explicit cancellation,
decommit/recommit and release/reuse. A managed TLS owner exits while a sibling
waits on its page; reaping must cancel that wait before returning the backing.
Enrollment is observed through wait usage rather than inferred from a sleep.

The kernel requires an actual idle HLT while a private word wait is pending,
then the main timed wait returns TimedOut. All eleven threads retire: the main
thread, two compiler-TLS children and eight word-test children. Existing SysV,
C/C++/Rust calls, three-thread compiler TLS and live YMM preemption proofs remain
required. Four committed/eight reserved pages stay live for final process
cleanup; both charges, every wait and every word registration must retire.

Boot-test builds additionally run trusted cleanup/admission fixtures. These
exercise retained pages after failed thread creation and retry invalidation;
they also require an already-expired word wait to beat native Wake admission.
An untimed word wait remains registered through interruptible HLT and a trusted
external timer IRQ, then completes by cancellation. These assertion markers are
separate from the Ring 3 ABI event. See the source and raw debug logs for the
trusted checks. Model tests cover key isolation across
spaces, remapping and vector movement, shared/RO/unpublished rejection, FIFO
detachment, single completion, metadata OOM and retained accounting. Callback
tests require admission and inactive-root validation before invalidation.

All 443 Rust host tests and 19 Python tests passed. Formatting, normal/all-feature
kernel Clippy, scoped changed core tests, HAL host targets, userspace workspace
and the custom-target ABI probe passed with warnings denied. Exact commands,
package counts and source hashes are in [host-validation.json](host-validation.json).

[evidence.json](evidence.json) records all six runs and packaged kernel/ELF/disk
hashes. [source-manifest.json](source-manifest.json) identifies source,
configuration and runner inputs, excluding documentation and generated files.
The runner stops its own QEMU after the required markers; host termination does
not establish guest exit.

The initial boot failed an existing exact frame-count assertion after shared
wait admission prepared an unused word-queue allocation. Preparation now occurs
only for matching Park calls, before wait publication. The old frame assertion
is unchanged. That excluded run is retained as `prior-eager-queue-*` artifacts.

## Limits

The kernel compares an owned-frame snapshot after all user CPU leases end; it
does not dereference arbitrary user pointers or form trusted Rust references
into user storage. The key includes unique space and mapping lifetime IDs plus
the exact word address. ELF data, managed stacks/TLS and explicit private VM
are supported; shared mappings are rejected. Pending-thread cleanup, ordinary
unmap, decommit/release and final teardown detach registrations before backing
retirement, including retries.

The policy remains serialized on the BSP. Word queues scan linearly, and repeated
per-wait removal can make a large completion batch quadratic. No contention
throughput, hardware latency, power or scalability result is claimed. The HLT
check establishes the timed wait's functional idle path, not physical C-states.
There are no shared waits, priority inheritance, requeue, robust mutexes,
multi-object atomic waits or complete pthread/libc/std integration.
