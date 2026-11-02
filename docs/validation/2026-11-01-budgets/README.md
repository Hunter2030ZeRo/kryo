# Service resource budget validation

Date: 2026-11-01.

All 16 final QEMU MTTCG configurations pass: default-model one-CPU 512-MiB guests,
max-model two-/four-CPU 512-MiB guests and max-model two-CPU 129-GiB guests, each in
debug/release and boot-tests/normal boot. `evidence.json` records commands, checks,
managed frames, execution mode and sampled resident memory. `source-manifest.json`
hashes 245 source/configuration/runner inputs, unchanged throughout the matrix.
Documentation, generated builds and validation artifacts are excluded.

Host tests pass 336 cases: core 261, HAL 58, interfaces 2, libsys 4 and services 11.
Raw output is in `host-tests.log`; `checks.log` records passing normal/all-feature
kernel Clippy, scoped changed core tests, userspace workspace Clippy and formatting.
Independent review found no blocker and separately ran 44 targeted model tests.
The existing whole-host-test range.rs identity_op remains outside that lint scope.
`prior-check-command.log` retains one invocation with a nonexistent test target;
the corrected target is capability_transfer and its final check passes.

## Real user execution

Each registry client completes three logged budget stages before ordinary service
requests. Both normal and boot-test runners require all six stage markers. Stage
values encode the service label (20 or 21) and stage 1..3; the labels authenticate
nothing and select no resource account.

1. The client duplicates its own endpoint until its shared 32-handle account is
   full. MemoryCreate cannot publish a result and Receive cannot reserve a new
   destination slot. Shared-page and wait usage stay unchanged. Closing duplicate
   handles restores capacity.
2. It allocates an eight-page shared payload, maps one page, closes the last
   handle and verifies all eight pages remain charged. Another allocation fails
   until the final mapping retires. A subsequent allocation succeeds.
3. Its initial thread plus three native threads consume four live-thread credits.
   Closing all new thread-control handles does not permit another live thread.
   Two workers fill the two-wait account. The third worker and main-thread sleep
   receive OutOfResources. Signal and Close remain usable. Workers exit and reap,
   leaving the initial thread credit only, with wait usage back to zero.

The new six threads actually execute; the boot harness requires six successful
ThreadCreate publications in this supervisor instance. Coordination uses atomic
state, bounded yield/clock checks and explicit notifications, rather than an
assumption that a short deadline will be accepted. The prior deterministic
accepted-cancellation/late-reply fixture, malformed registration, independent
channel-account exhaustion, reduced discovery grants and replacement still pass.

## Ownership and authority checks

Core models cover overflow/exhaustion, independent accounts, concurrent charging,
open/reserved handle accounting, forgotten reservation tokens, unused commits and
allocation-null rollback. Ready waits retain credits until consumed. Memory tests
retain shared-page charges after creator destruction, mapping/unmapping failure,
forgotten arena references and a synthetic failed backing return during host
unwind. VM models bound aliases and root-inclusive private table allocation.

Trusted guest fixtures exercise assignment permission/privacy and stale sources,
legacy-spawn inheritance, thread limits after control closure, Ready wait credits,
failed thread cleanup and post-load publication failure with repeated restoration
failure. Pending owners keep their thread charge until cleanup retry succeeds.
A shared object survives creator process reaping through a different process's
mapping, then releases its page credit when that final mapping ends.

Normal boot gives the supervisor ten explicit root grants and its own account.
Service accounts are selected through ASSIGN|TRANSFER and remain the same across
restart. Leaves receive no assignment capability. Self inspection does not grant
new authority. Test account policy is shared_pages=8, handles=32, waits=2,
threads=4, with per-process mappings=64 and tables=32. Root policy is 64/128/16/16
with mappings=128 and tables=64. These are configured account policies, not global
kernel process/thread caps. The existing channel accounts are separate.

Boot tests require zero remaining objects, waits and resource charges after all
owners retire. Physical-frame counts restore except for measured retained kernel
HHDM table/metadata growth. Normal boot instead observes the persistent supervisor
and active replacement after the user checks. Legacy regressions keep their
trusted unbudgeted constructors; both production spawn paths inherit a binding
unless explicit assignment selects another account.

| Configuration | Mode | Peak RSS MiB | Result |
| --- | --- | --- | --- |
| debug-default-cpu1-512M | boot-tests | 122.63 | passed |
| release-default-cpu1-512M | boot-tests | 101.59 | passed |
| debug-default-cpu1-512M | normal | 101.44 | passed |
| release-default-cpu1-512M | normal | 84.42 | passed |
| debug-max-cpu2-512M | boot-tests | 126.52 | passed |
| release-max-cpu2-512M | boot-tests | 104.57 | passed |
| debug-max-cpu2-512M | normal | 106.46 | passed |
| release-max-cpu2-512M | normal | 87.08 | passed |
| debug-max-cpu4-512M | boot-tests | 131.55 | passed |
| release-max-cpu4-512M | boot-tests | 107.45 | passed |
| debug-max-cpu4-512M | normal | 111.36 | passed |
| release-max-cpu4-512M | normal | 92.36 | passed |
| debug-max-cpu2-129G | boot-tests | 150.68 | passed |
| release-max-cpu2-129G | boot-tests | 128.62 | passed |
| debug-max-cpu2-129G | normal | 122.38 | passed |
| release-max-cpu2-129G | normal | 103.64 | passed |

## Scope and reproduction

The static image page budget increased from eight to a named 16 pages after the
budget-aware supervisor measured nine debug load pages. The eight-load-segment,
64-program-header and one-MiB module limits remain. Scheduling quanta, one-second
RPC deadlines and bootstrap watchdogs were unchanged. All twenty debug/release
boot ELFs retain their entry and load bytes after debug stripping; packaging
hashes are recorded in `packaging.json`.

These are live-resource counts and per-process VM bounds. They are not exact
physical-RAM accounting: private page/table limits, kernel heap metadata, retained
HHDM splits and shared stack-region tables need separate accounting. Private
user page tables can remain allocated after a leaf unmap. CPU-time budgets and
per-service scheduling fairness are not implemented; ticks count preemptions.
The current scheduler is per-thread round-robin and kernel policy is serialized
on the BSP. More runnable threads can increase a service's share within its limit.

SIMD support is unchanged: eager per-thread x87/SSE/optional AVX preservation,
with the bounded save layout and kernel soft-float. Kernel SIMD use, AVX-512,
heterogeneous CPU policy, physical latency/energy measurements, general device/DMA
support, ICE, BetAFS I/O and application integration are outside this evidence.

Run `python3 tools/boot_smoke.py --profile debug|release`, optional `--normal`,
`--cpu max --cpus 2|4`, and `--memory 129G` for the large two-CPU selection. Runs
retain the existing 90-second host deadline and 2048-MiB RSS guard and stop only
their own emulator after markers. The peak sampled RSS was below 151 MiB.
129-GiB guests are demand-backed and sparsely touched; this is not a full RAM
sweep, guest shutdown or physical-hardware proof.
