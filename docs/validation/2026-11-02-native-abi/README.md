# Freestanding native language ABI

Date: 2026-11-02.

Six final QEMU TCG cases passed. Debug/release Haswell-noTSX boot-test kernels
ran the new mixed-language probe on one CPU. Debug/release normal kernels ran
it on four max CPUs with writable/read-only scratch block devices respectively.
Two additional four-CPU runs omitted the probe: debug boot tests and release
normal boot. Those images contain no ABI probe module and emit no ABI marker.

The enabled probe validates owned argc/argv/envp/auxv, a separate read-only
startup capability, C/Rust/C++ mixed hard-float calls, stack arguments, C varargs
and aggregate returns. Live YMM0/YMM15 patterns survive yield/sleep. Start/end
events inside the assembly block bracket live vectors; increased thread ticks
between them and final output validation establish timer-preempted AVX2 state
preservation in this single-runnable-task fixture. All enabled runs require
startup and execution-resource retirement afterward.

Both debug/release ELFs fit the unchanged 16-load-page policy. Per-run compiler
metadata, executable hashes and load-page counts are retained in `evidence.json`
and adjacent `.build.json` files. Target and lock hashes match current source.
Kernel staging copies match their run's private target artifact; the two block
images match their complete expected hashes.

Host tests passed: core 303, HAL 77, interfaces 7, libsys 4 and services 11
(402 total). Seven new core tests cover stack layout, strings, alignment,
capacity, invalid inputs and unchanged output on failure. The Python parser
passed 14 tests, including preemption/retirement markers and separate ABI labels.
Kernel normal/all-feature, scoped core stack and custom-user-target Clippy with
warnings denied, plus Rustfmt, passed. Broader core all-target host Clippy found
an existing unrelated identity-op warning in `tests/range.rs`; that fixture was
not changed or suppressed.

One earlier parallel verification run timed out with normal-service output where
boot-test output was expected. The old recipes shared the kernel profile artifact
between build and ISO copy. `prior-shared-target-*` preserves that failed run.
The corrected runner isolates kernel targets by run label. The six final cases
were executed with this fix, including parallel normal and boot-test builds.

`source-manifest.json` records 278 active source/build inputs. Host logs and
guest debug/stdout/stderr logs are adjacent. Host termination of the owned
emulator is not guest shutdown. QEMU timing/preemption is functional evidence,
not a physical latency, throughput, scaling or power measurement.

The validated contract is a finite freestanding bootstrap. It supplies no general
crt0, mapped ELF metadata, ELF TLS, libc, libc++, Rust std, dynamic linker or
Intel GPU support. See [ADR 0024](../../decisions/0024-native-bootstrap-stack.md).
