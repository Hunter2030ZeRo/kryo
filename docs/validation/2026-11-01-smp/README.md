# SMP validation

The final QEMU matrix passed 24 configurations: 1/2/4 CPUs, debug/release,
default/max CPU models, and boot-tests/normal boot. Commands use multi-threaded
TCG and a 30-second host deadline. Evidence records each command, check, log,
and host termination. It does not claim physical hardware, performance, power,
or guest shutdown validation.

Boot tests keep the previous BSP regressions, then initialize AP timers and run
the SMP path. Checks cover fresh same-process barriers and counters on distinct
CPUID identities, actual thread migration with private FS/TLS, sleep/wakeup,
fatal process policy before a sibling's trapped syscall, and remote TLB alias
replacement. Injected ACK loss verifies retained backing, blocked slot reuse and
dispatch, successful retry, and stack-publication rollback. Full cleanup and AP
retirement precede region removal and final ABC. The 1-CPU path explicitly skips
parallel-only checks.

Normal boots use the same scheduler and run compiled init/worker programs.
Each worker creates two successive threads and checks shared data/handles,
private TLS/x87/SSE, and optional AVX state across preemption and joins. Init
observes two exit-42 generations and retains the old process control.

The source manifest identifies the staged source used by the matrix. Installation
compares those hashes with the actual checkout. This is source-equivalence
evidence; emulator logs retain their original staging command paths.

All 254 host tests pass: core 207, HAL 46, interfaces 1. Kernel workspace Clippy
passes with and without boot-tests; changed core task targets and all HAL host
targets pass with warnings denied. Whole-workspace host `--all-targets` Clippy
also checked an unchanged test and rejected `kernel/core/tests/range.rs:43`
for `identity_op` (`u64::MAX & !(FRAME_SIZE - 1)`). That pre-existing lint remains;
it does not affect the passing test or kernel builds.

The initial AP instruction-fetch fault and migration failure are retained beside
the final logs. APs now leave Limine's HHDM trampoline before NX protection is
enabled. Notification retirement, batch release, queued-IPI drain, and fresh
entry-time quanta prevent old wakeups from repeatedly ending a new slice before
progress.
