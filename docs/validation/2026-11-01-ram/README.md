# Runtime frame metadata and large RAM validation

Date: 2026-11-01.

All 20 final configurations pass: max-model two-CPU guests at 512 MiB, 8 GiB,
128 GiB and 129 GiB, plus default-model one-CPU 512-MiB guests, each in debug/release
and boot-tests/normal boot. `evidence.json` records the commands, checks, managed
frame counts, selected physical addresses, sampled resident memory and scoped
host termination. `source-manifest.json` hashes tested source/runner inputs;
Markdown, generated builds and validation artifacts are excluded. The source
inputs stayed unchanged throughout the final matrix.

Host tests pass 298 cases: core 239, HAL 58, interfaces 1. `host-tests.log` retains
the complete output. New pool tests cover exact 128-GiB metadata sizing, a larger
sparse plan at high physical addresses, external-buffer failure/retry without
mutation, one-time publication, reserved ranges and targeted allocation. These
host models allocate metadata and tokens; they do not allocate 128 GiB of payload.
Kernel normal/all-feature Clippy, scoped host core pool Clippy and formatting pass.
The earlier full-host-test Clippy identity_op at range.rs:43 remains outside that
clean host-test lint scope.

The production frame pool uses external buffers carved from normalized Usable
RAM before the heap starts. The page-rounded span is reserved and normalization
runs again. Guest checks require exact removed-frame accounting, no overlap with
remaining available ranges, sufficient bitmap/summary words and failed allocation
from the metadata reservation. A bootstrap one-time guard precedes raw zeroing;
typed slices are created only after initialization. Exactly 128 GiB of Usable RAM
needs 4 MiB of bitmap plus 64 KiB of summary before page rounding.

Boot tests use real pool-owned pages at or above 4 GiB and 128 GiB where present.
They write/read through scratch and HHDM aliases, zero, retire translations,
release, reallocate the exact page and restore counts. Remote TLB and missing-ACK
checks prefer high physical backing; final large-guest logs record distinct old
and new pages at 0x2000000000 and 0x2000001000. Every 129-GiB guest manages more
than 128 GiB after reservations. Existing user isolation, FP/TLS, IPC, device,
process/thread growth and teardown checks still pass.

QEMU uses `memory-backend-ram` with prealloc=off/reserve=off and mem-lock=off.
The runner samples its own emulator's VmRSS/VmHWM and stops it above a 2048-MiB
bound. Peak sampled process RSS stayed below 144 MiB in this matrix. See
[QEMU memory backends](https://www.qemu.org/docs/master/system/qemu-manpage.html).
Guest size, available/managed frames, physical addresses and host resident memory
are separate observations. This is sparse functional validation, not a complete
write sweep, physical DRAM test, performance result or memory-hotplug proof.

| Configuration | Managed GiB | Peak process RSS MiB | Result |
| --- | --- | --- | --- |
| debug-max-smp2-ram129G | 128.96566 | 143.06 | passed |
| release-max-smp2-ram129G | 128.97308 | 124.54 | passed |
| debug-normal-max-smp2-ram129G | 128.96770 | 118.10 | passed |
| release-normal-max-smp2-ram129G | 128.97459 | 100.96 | passed |
| debug-max-smp2-ram128G | 127.96569 | 142.86 | passed |
| release-max-smp2-ram128G | 127.97311 | 124.42 | passed |
| debug-normal-max-smp2-ram128G | 127.96774 | 118.51 | passed |
| release-normal-max-smp2-ram128G | 127.97462 | 101.09 | passed |
| debug-max-smp2-ram8G | 7.96941 | 121.72 | passed |
| release-max-smp2-ram8G | 7.97683 | 104.92 | passed |
| debug-normal-max-smp2-ram8G | 7.97145 | 102.53 | passed |
| release-normal-max-smp2-ram8G | 7.97834 | 85.77 | passed |
| debug | 0.47013 | 114.25 | passed |
| release | 0.47731 | 96.80 | passed |
| debug-normal | 0.47218 | 96.67 | passed |
| release-normal | 0.47882 | 82.12 | passed |
| debug-max-smp2 | 0.46964 | 119.29 | passed |
| release-max-smp2 | 0.47707 | 100.62 | passed |
| debug-normal-max-smp2 | 0.47169 | 101.97 | passed |
| release-normal-max-smp2 | 0.47858 | 85.02 | passed |

## Preliminary fixture correction

`prior-low-reservation.*` retains the first 129-GiB attempt. Its metadata and high
frame probes passed, then the existing alias-preparation rollback assertion did
not observe the expected TableCapacity error. The low-memory fixture had relied
on the global allocation cursor walking addresses in order. High probes can move
that cursor. The fixture now explicitly selects [0, 2 MiB) with allocate_in,
retaining its original expected backend failure and exact cleanup assertions.
Scheduling quanta, deadlines and execution budgets did not increase. These prior
artifacts belong to an intermediate source and do not attest final-source results.

Reproduce with `python3 tools/boot_smoke.py --memory 129G --cpus 2 --cpu max`,
`--profile debug|release`, and optional `--normal`. The one-CPU default selection
uses 512M and omits --cpu. The runner stops only its own emulator; this is not a
guest shutdown test. The interactive Makefile accepts MEMORY=128G separately.
