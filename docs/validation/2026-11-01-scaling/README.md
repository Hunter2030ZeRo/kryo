# Registry and CPU-set scaling validation

Date: 2026-11-01.

`evidence.json` records the final standard MTTCG matrix. Its debug/release
boot-tests and normal boots use max CPU at 2, 8 and 65 processors, and default/max
models at one processor. The runner retains the 90-second host boot deadline,
stops only its own QEMU process, and requires every marker for the selected mode.
Host termination is not guest shutdown.

`source-manifest.json` identifies the final tested source inputs. `host-tests.log`
records 291 passing tests (core 232, HAL 58, interfaces 1). Kernel normal/all-feature
Clippy and formatting pass; production userspace workspace Clippy and formatting
pass. Scoped host core checks (lib, growth, allocation_failure, vm, pool) and all
HAL targets pass Clippy with warnings denied. A pre-existing identity_op in
`kernel/core/tests/range.rs:43` remains outside that clean host-test lint scope.

The configured xAPIC registry has 255 logical slots. Host models cover indices
63/64/127/128/254, sparse APIC identities, independent leases and missing high-word
acknowledgements. The final positive guest inventory reaches 65 processors;
128/255 guest operation and physical hardware are unconfirmed. No x2APIC,
throughput, latency, power or hardware-stack-bound result is claimed.

The native scale fixture starts six children from one parent; each child creates
20 threads. It requires seven live processes, 127 live threads, 127 registered
native timed waits, distinct TLS state across a yield, and growth from 512 to
1024 stack-region pages. All waits and process/thread resources then retire.
An injected process-membership allocation failure preserves the unpublished
owner and exact frame accounting, and a retry succeeds. Final accounting retains
exactly the kernel-owned HHDM split tables and their heap metadata; it must restore
all remaining frames. Stack-region table owners return only after final global
retirement. Registry generations persist until their registry owners drop.

The boot-only fault/call policy fixture captures independent actual UD2 and
int 0x80 return causes without early peer interruption. It retains the existing
two-millisecond slices and watchdog, then checks RIP, terminal state and debug-call
suppression after process death. Production batch scheduling is unchanged.

## Preliminary diagnostics

Files named `prior-*` belong to intermediate source/configuration experiments and
are not evidence of final-source success. No exact source-equivalence claim is
made for those builds. Earlier 128-processor MTTCG execution ended with SIGKILL
before a kernel log; 255 timed out after 90 seconds with an empty log. A separate
controlled 255-CPU QMP probe observed BSP CS16/CR3=0 at ten seconds and CS32/CR3=0
at thirty seconds, outside Kryo long-mode text. These snapshots locate that probe;
they do not establish the cause of the earlier failures. No OOM event was found.
All register samples describe BSP CPU0: the monitor request omitted cpu-index.

Serialized TCG experiments produced timer-vector 0xe0 preemption before UD2;
instruction-clock variants timed out before kernel logging. A 65-CPU experiment
with a 64-MiB TB cache failed an early progress assertion; final standard-cache
runs passed. These observations do not isolate a cache or host-pressure cause.
The experimental serialized lane and its options are not shipped. QEMU documents
[instruction counting](https://www.qemu.org/docs/master/devel/tcg-icount.html) as
virtual-time accounting rather than cycle-accurate hardware simulation.

Reproduce the final selections with `tools/boot_smoke.py --cpus N --cpu max`,
`--profile debug|release`, and optional `--normal`. One-CPU runs also omit `--cpu`
for the default model. Dependencies and output locations are documented in
`tools/README.md`.
