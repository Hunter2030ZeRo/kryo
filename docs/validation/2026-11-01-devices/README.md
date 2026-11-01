# Device authority and CPU expansion validation

The combined matrix selects 1/2/8/64 logical CPUs. Default and max CPU models run
at 1/2; max runs at 8/64. Each selection runs debug/release and normal/boot-tests,
for 24 configurations. Commands use MTTCG, `hpet.msi=on`, and a 90-second host limit.
Evidence retains commands, checks, logs, and source hashes. Host termination is
not guest shutdown. No physical hardware, power, or performance claim is made.

Host tests pass 272 cases: core 215, HAL 56, interfaces 1. Kernel Clippy passes with
and without boot-tests; changed core task/device targets and all HAL host targets
pass with warnings denied. User ELF builds and production Clippy pass. The
pre-existing whole-host-test Clippy `identity_op` in `range.rs:43` remains outside
this patch. Bare-target user `--all-targets` is not a supported test harness.

The device ELF validates aligned register access, denied registers/configuration,
ordinary/timed real-IRQ waits, armed process stop, and IRQ-consumer ThreadExit
while a sibling keeps the process alive. Epoch changes prove actual D0 delivery;
the driver separately checks counter progression and filters redundant wakeups.
The normal bootstrap runs four cases, not a persistent device-manager service.

Boot-only cases verify actual queued D0 retirement, failed cleanup with retained
owners/denied reuse, successful retry, and channel registration exhaustion. The
expiry fixture combines a directly constructed core wait with real HPET delivery:
an already expired registration returns TimedOut, and a fresh wait consumes the
retained late signal without another IRQ. Native positive wait calls are tested
separately by the ELF. These are not PCI/DMA retirement or user-MMIO map proofs.

CPU tests cover sparse APIC identities, logical slot 63, physical ID 254, broadcast
rejection, online epochs, shared-root execution and migration, remote TLB flushes,
and collective AP shutdown. Four scheduled-thread/two-process limits remain
separate from 64-CPU registration. `cpu-storage.json` distinguishes fixed storage
estimates from actual-CPU Limine stacks; it is not a measured stack high-water.

Initial 64-CPU runs exposed a busy early AP parking loop and order assumptions in
legacy fixtures. Prepared APs now use interruptible HLT through a retained static
APIC branch. Alias policy explicitly validates that extra MMIO-only branch.
Scalar FP testing permits preemption but still requires an actual #NM. Notification
coalescing waits for producer completion rather than counting yields. Timed IPC
actors use causal notifications/control-state checks rather than relative sleeps
to establish queued, accepted, retry, and cancellation states. Deadlines, expected
results, scheduler quanta, and bounded execution budgets are unchanged. Queued
timeout cases require observation of a real wait rather than immediate-expiry-only
evidence. Earlier logs remain alongside final results; not every old intermittent
failure was conclusively attributed to one assumption.

An early normal-boot runner counted a bootstrap probe thread as an init worker.
The retained `prior-runner` log shows actual ready/exit markers despite that
false negative. Supervision parsing now starts at init dispatch. Fault collection
also leaves a short bounded interval for complete panic diagnostics before
stopping its emulator.
