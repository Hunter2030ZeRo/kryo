# Development tools

The root [Makefile](../Makefile) provides build, ISO, host-test, and QEMU targets.

`boot_smoke.py --pci-probe` attaches a disposable read-only modern VirtIO block
device behind a configured PCI bridge. It checks nonzero-bus discovery, inventory
counts and unchanged command bits. The raw fixture has no filesystem; no block
request is submitted. Its hash is checked again after the owned emulator stops.
Ordinary runs use a separate output directory and also require the PCI inventory
and unpublished DMA-buffer checks. Run `python3 -m unittest discover -s tools`
for the evidence parser tests. See [storage validation](../docs/validation/2026-11-02-storage/README.md).

boot_smoke.py --block-probe opts in to real Ring 3 block commands on a fresh
signed scratch disk. Add --block-readonly to require a device write error and
unchanged bytes. The runner passes BLOCK_PROBE=1 to packaging, verifies the whole
expected image after QEMU stops and uses separate -block-rw/-block-ro output
directories. Ordinary images omit the enable marker. The guest verifies the disk
signature before writing. No host block device is attached.
See [block validation](../docs/validation/2026-11-02-block/README.md).

`build_user.py` builds the separate `userspace/native-smoke` package with isolated small-code-model static user flags. It clears inherited Rust flags and writes a static ET_EXEC ELF; it does not link the kernel, Linux, NT, std, or dynamic libraries.

```sh
make user
python3 tools/build_user.py --profile release --output build/native-smoke.elf
```

`make iso` builds the kernel and thirteen freestanding Rust boot images.
The normal image starts the configured supervisor; boot tests additionally run
the legacy fixtures. Module selection rejects missing, duplicate, empty or
oversized images. `build_user.py --program` selects an image; the default remains
native-smoke.

## Native language ABI probe

`python3 tools/check_c_headers.py --outputdir /tmp/kryo-c-headers` checks the
experimental public C profile and native call header against the owned registry
and Rust constants. It uses no host headers for target checks. Its separately
labeled Linux-host executable tests only integer wait macros; it makes no
native calls and does not establish a libc port.

`make iso ABI_PROBE=1` additionally builds the mixed C/C++/Rust probe with the
isolated x86-64-v3 hard-float target. `make abi` builds just that ELF. This path
needs Clang/Clang++ and rust-src matching the validated Rust revision in
`targets/cryolite-toolchain.json`; it does not change ordinary kernel/user flags.
The builder rejects a different compiler revision and defaults to locked offline
dependencies. `tools/build_abi.py --fetch-dependencies` explicitly permits the
initial download of missing Rust sysroot dependencies.

```sh
python3 tools/boot_smoke.py --profile debug --cpu Haswell-noTSX --abi-probe
python3 tools/boot_smoke.py --profile release --cpu max --cpus 4 --normal --abi-probe
```

The output adds `-abi`. Checks require C/Rust/C++ mixed calls, owned argc/argv/
envp/auxv, separate version-1 startup authority, live YMM state across native
waits and a timer-preempted AVX2 interval, followed by complete resource retirement.
Ordinary runs omit the module and skip the probe before allocating resources.
This is not libc, libc++ or Rust std support. See
[the bootstrap contract](../docs/decisions/0024-native-bootstrap-stack.md).

The probe now also requires truthful AT_PHDR/PHENT/PHNUM and native local-exec
TLS in C, C++ and Rust. Runtime-owned private slots are initialized from the
read-only ELF TLS template; main and two native children verify independent
initialized/BSS values across yield/sleep/join. The final kernel check retires
all three execution owners. See
[static TLS](../docs/decisions/0025-static-tls-and-elf-metadata.md) for the aligned
fixture, minimal TCB and legacy scratch-mapping limits.

The C peer directly uses the [native SDK](../userspace/libsys/c/README.md) for
memory, clocks, private waits and bounded diagnostics. The C ABI startup fixture
now carries a high-bit argument and WRITE|DUP debug authority; its temporary
zero-rights duplicate must fail diagnostic admission. The original supervisor
and ordinary service grants are unchanged. Four escaped USERDIAG records and
the final kernel CABI retirement marker are required; user payloads are excluded
from trusted kernel-marker matching. The new output goes to the debug log,
independently of the early framebuffer rectangle.

Mapped ELF headers change the packaging comparison: only ELF64 section-table
bookkeeping is normalized when checking this probe. Entry, program headers and
all other LOAD bytes remain exact. The thirteen earlier boot images retain the
strict comparison.

Each smoke label uses a private kernel target directory so parallel normal and
boot-test recipes cannot replace each other's kernel before ISO copying.

`boot_smoke.py` builds a test ISO and runs it once under headless QEMU TCG:

```sh
python3 tools/boot_smoke.py --profile debug
python3 tools/boot_smoke.py --profile release
python3 tools/boot_smoke.py --all
python3 tools/boot_smoke.py --profile debug --cpu max
python3 tools/boot_smoke.py --all --cpu max
```

`--all` runs debug and release once each for the selected CPU. Without `--cpu`,
the runner explicitly selects `max`; `make run` also defaults to `CPU=max`.
Use `--cpu Haswell-noTSX` to exercise an earlier v3-capable CPU model. Explicit
models keep separate output directories. `qemu64` lacks the required baseline
and is rejected during early boot; it cannot pass an ordinary smoke run.

Required markers cover memory allocation/zeroing, mapping and VM rollback, 16 user-isolation cases, eight timer-driven switches, capability rejection, invalid saved-state isolation, pending-interrupt drain, and full task teardown.

IPC adds eight rendezvous/cancellation/failure/notification cases and three BSP halt/wakeup cases. Shared-memory markers require three shared pages and two reduced-rights capability grants. The receiver checks denied write amplification and redelegation, then uses its mappings after both sets of handles close and the producer unmaps its pages. A separate two-grant/zero-capacity case checks rollback followed by a valid message without capabilities. Acceptance checks require grant commit before cleanup of an earlier sender completion and complete object/frame reclamation.

An additional shared-RO write must fault with vector 14/error 7, isolate the task, and reclaim the last mapping. These programs exercise experimental native calls 14-18 alongside the 32-byte inline protocol. Shared mappings are non-executable; DMA loans, move transfer, RECEIVE/WAIT owner migration, and general service discovery remain pending. The run ends with the ABC breakpoint sequence.

Native ELF markers require two instances in independent address spaces, actual timer preemption of both, data/BSS checks, distinct x87/SSE/TLS state across yields and timer returns, and complete frame reclamation. Startup passes instance 41/42 in RDI, a private TLS page in RSI, and an AVX flag in RDX bit 0. Call 19 sets the validated FS base; the sample supplies manual TLS storage, and PT_TLS remains rejected. Rust code remains soft-float and assembly supplies the SIMD probes.

All boots require exactly one v3 hardware admission marker per participating
CPU. Boot tests also require the `avx_context` check: native execution must
report `xsave=true avx=true`, and the sample must preserve a YMM upper half.
The kernel uses a private 64-byte-aligned, 1024-byte standard state area per
extended context. Earlier FXSAVE support remains internally tested but is not
a fallback for unsupported platform hardware. A scalar-mode `fldz` check must
still produce #NM after native execution. These are CPU-state checks for this
static native fixture, not general std/C ABI or dynamic-loader support.

Each profile has a 90-second host boot deadline. Logs and JSON evidence are saved under `build/smoke/debug/` or `build/smoke/release/` for the default CPU, and `build/smoke/debug-max/` or `build/smoke/release-max/` for `--cpu max`. Explicit qemu64 runs use the corresponding `-qemu64` suffix. Failure output is preserved; the runner does not retry automatically. It stops only its own QEMU process after the checks or deadline.

Clock markers require checked ACPI discovery, a progressing 64-bit HPET counter, and local APIC calibration. Deadline markers require Ring 3 sleep and timed IPC, delivery-state preservation, cancellation, stale-event handling, and complete cleanup. Guest user slices and idle use calibrated one-shot events; legacy pending-IRQ regressions still use raw counts. The host deadline is independent of the guest clock. STI/HLT/CLI checks establish guest wakeup behavior, not physical power savings or a real-hardware latency bound. Host termination of QEMU does not prove guest shutdown.

`xtask/` and `debug/` are reserved for future tooling.

`--normal` checks a kernel built without `boot-tests`. Separate `debug-normal`/`release-normal` evidence (with an optional CPU suffix) requires actual user logs, two different child generations, two observed exit codes, and init readiness. Regression mode also requires launch rollback and supervision teardown markers. Both modes have the same 90-second host deadline and use the common kernel runtime.

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

The block selection also checks the initial IPC service and independent clients.
The first client has its accepted read cancelled, then awaits an Info barrier
before reusing memory. Later clients test read/write authority and scratch block
2 with pattern ((offset * 17 + 0x53) & 255). Normal mode retains the driver;
boot tests request graceful Stop and require all resource accounts to restore.
See [service validation](../docs/validation/2026-11-02-block-service/README.md).
