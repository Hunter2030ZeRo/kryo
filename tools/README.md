# Development tools

The root [Makefile](../Makefile) provides build, ISO, host-test, and QEMU targets.

`build_user.py` builds the separate `userspace/native-smoke` package with isolated small-code-model static user flags. It clears inherited Rust flags and writes a static ET_EXEC ELF; it does not link the kernel, Linux, NT, std, or dynamic libraries.

```sh
make user
python3 tools/build_user.py --profile release --output build/native-smoke.elf
```

`make iso` builds the kernel and four user ELFs. Limine tags are `kryo-init`, `kryo-worker`, `kryo-native-smoke`, and `kryo-device-probe`. Normal boot selects init and worker; boot tests additionally use native-smoke. Module selection rejects missing, duplicate, empty, or oversized images. `tools/build_user.py --program init` and `--program bootstrap-worker` build the new workspace; the default program remains native-smoke.

`boot_smoke.py` builds a test ISO and runs it once under headless QEMU TCG:

```sh
python3 tools/boot_smoke.py --profile debug
python3 tools/boot_smoke.py --profile release
python3 tools/boot_smoke.py --all
python3 tools/boot_smoke.py --profile debug --cpu max
python3 tools/boot_smoke.py --all --cpu max
```

`--all` runs debug and release once each for the selected CPU. Without `--cpu`, QEMU selects its default CPU. `--cpu qemu64` and `--cpu max` select explicit models and keep separate output directories.

Required markers cover memory allocation/zeroing, mapping and VM rollback, 16 user-isolation cases, eight timer-driven switches, capability rejection, invalid saved-state isolation, pending-interrupt drain, and full task teardown.

IPC adds eight rendezvous/cancellation/failure/notification cases and three BSP halt/wakeup cases. Shared-memory markers require three shared pages and two reduced-rights capability grants. The receiver checks denied write amplification and redelegation, then uses its mappings after both sets of handles close and the producer unmaps its pages. A separate two-grant/zero-capacity case checks rollback followed by a valid message without capabilities. Acceptance checks require grant commit before cleanup of an earlier sender completion and complete object/frame reclamation.

An additional shared-RO write must fault with vector 14/error 7, isolate the task, and reclaim the last mapping. These programs exercise experimental native calls 14-18 alongside the 32-byte inline protocol. Shared mappings are non-executable; DMA loans, move transfer, RECEIVE/WAIT owner migration, and general service discovery remain pending. The run ends with the ABC breakpoint sequence.

Native ELF markers require two instances in independent address spaces, actual timer preemption of both, data/BSS checks, distinct x87/SSE/TLS state across yields and timer returns, and complete frame reclamation. Startup passes instance 41/42 in RDI, a private TLS page in RSI, and an AVX flag in RDX bit 0. Call 19 sets the validated FS base; the sample supplies manual TLS storage, and PT_TLS remains rejected. Rust code remains soft-float and assembly supplies the SIMD probes.

The default CPU can use the FXSAVE fallback. `--cpu max` additionally requires the `avx_context` check: the native execution marker must report `xsave=true avx=true`, and the sample must preserve a YMM upper half. The kernel uses a private 64-byte-aligned, 1024-byte state area per extended context and allows only x87/SSE plus bounded optional AVX. A scalar-mode `fldz` check must still produce #NM after native execution. These are CPU-state checks for this static native fixture, not a general std/C ABI or dynamic-loader claim.

Each profile has a 90-second host boot deadline. Logs and JSON evidence are saved under `build/smoke/debug/` or `build/smoke/release/` for the default CPU, and `build/smoke/debug-max/` or `build/smoke/release-max/` for `--cpu max`. Explicit qemu64 runs use the corresponding `-qemu64` suffix. Failure output is preserved; the runner does not retry automatically. It stops only its own QEMU process after the checks or deadline.

Clock markers require checked ACPI discovery, a progressing 64-bit HPET counter, and local APIC calibration. Deadline markers require Ring 3 sleep and timed IPC, delivery-state preservation, cancellation, stale-event handling, and complete cleanup. Guest user slices and idle use calibrated one-shot events; legacy pending-IRQ regressions still use raw counts. The host deadline is independent of the guest clock. STI/HLT/CLI checks establish guest wakeup behavior, not physical power savings or a real-hardware latency bound. Host termination of QEMU does not prove guest shutdown.

`xtask/` and `debug/` are reserved for future tooling.

`--normal` checks a kernel built without `boot-tests`. Separate `debug-normal`/`release-normal` evidence (with an optional CPU suffix) requires actual user logs, two different child generations, two observed exit codes, and init readiness. Regression mode also requires launch rollback and supervision teardown markers. Both modes have the same 30-second host deadline and use the common kernel runtime.

Thread markers require nine lifecycle/retirement cases and successful private-state checks in a shared process. Normal boots additionally require four distinct created thread generations, two within each worker process. Debug metadata construction uses a 512-KiB Limine entry stack per CPU; release uses 256 KiB. Per-thread entry stacks remain guarded 16-KiB mappings. These checks do not establish hardware stack high-water limits.

Use `--cpus 2`, `4`, `8`, `16`, `32`, `64`, `128`, `192`, or `255` to exercise SMP under multi-threaded TCG. The output directory adds `-smp2` or `-smp4`; evidence records the actual CPU count and command. Boot checks require CPU registration, same-process parallel execution, remote TLB retirement, and final AP retirement. `--cpus 1` preserves the local execution regression and explicitly skips parallel-only assertions. `make run` defaults to four guest CPUs; override with `CPUS=1` when needed.

The device probe uses `-global hpet.msi=on` for a real FSB IRQ without PCI bus mastering. All normal/boot-test runs include four user probe cases: untimed/timed waits, armed process stop, and consumer-thread exit. Boot tests additionally check queued-vector draining, failed retirement retry, expiry before deferred IRQ delivery, and fallible channel registration. The expiry fixture creates its parked core wait directly; positive native wait calls are exercised separately by the ELF. Normal supervision markers are parsed only after init starts, so bootstrap probe threads cannot inflate worker-thread counts.

Up to 255 xAPIC logical CPUs can be online; normal process/thread registries and their wait/handle storage grow through fallible allocation. x2APIC is not implemented. Prepared APs use HLT on an early retained APIC branch, and shutdown waits for all offline acknowledgements together. Increasing the CPU count does not establish hardware scaling or power results.

Boot checks load `scale-probe.elf` and require seven concurrent processes, 127 threads, 127 real timed waits, unique TLS state, stack mappings beyond the first 2 MiB leaf, and full execution-resource retirement. An injected metadata allocation failure must preserve the unpublished process and permit retry. Bounded old fixtures still exercise exhaustion deliberately. These checks establish functional growth, not scheduling throughput or physical-hardware scalability.

Guest RAM defaults to 512 MiB. Use `--memory 128G` or `--memory 129G` with the
smoke runner; `make run MEMORY=128G` selects the interactive guest size. Both use
demand-backed anonymous RAM with preallocation and reservation disabled. The
smoke runner samples its own emulator's resident memory and stops it above the
`--rss-limit-mib` bound (2048 MiB by default). The interactive Makefile path does
not apply that runner bound.

RAM checks require exact metadata reservation/accounting, high-frame HHDM and
scratch-alias access when those physical ranges exist, and high backing in the
TLB-retirement fixture. Evidence includes requested guest bytes, managed frames,
physical probe addresses, and sampled peak RSS. The 129-GiB matrix additionally
requires at least 128 GiB of managed frames after reservations. These tests do
not write every guest page or establish physical DRAM reliability.
