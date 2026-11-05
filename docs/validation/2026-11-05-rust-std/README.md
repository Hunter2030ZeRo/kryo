# Restricted native Rust std validation

The pinned Rust standard library now executes in Ring 3 through the Cryolite
runtime. This record covers the [experimental profile](../../../userspace/rust-std/README.md),
not a complete std/POSIX port or upstream target support.

| Configuration | Result | Checks | Packaged std image |
| --- | --- | --- | --- |
| Debug, Haswell-noTSX, one CPU, boot tests + ABI/libc/std | Passed | 86 | 410,472 bytes |
| Release, max, four CPUs, normal supervisor + ABI/libc/std + disposable block read/write | Passed | 46 | 286,008 bytes |

The two configurations used separate source/output trees. After hash-checked
application to /home/entity27th/kryo, the actual repository also rebuilt the std
executable successfully; actual-verification.json records that separate build. The archived build
metadata records compiler/Cargo/source pins, generated overlays, actual probe
sources, v3/no-MMX code generation and executable hashes. Packaging verifies
mapped bytes before and after debug stripping; the one-MiB module limit remains.

Each guest runs two normal std process instances. They check startup arguments,
Vec/String, aligned allocation, failed 64-MiB allocation under a 32-MiB explicit
private-VM quota, then successful 64-KiB allocation. The main thread holds a
Mutex while children enter acquisition; the kernel requires a live private-word
wait before release. Condvar timeout/notification, joined return values, joined
and detached compiler-TLS destructors, detached allocation retirement and
monotonic sleep complete before successful exit. The second instance has only
one wait credit, exercising the quota fallback.

A third fresh process calls std::process::abort. The kernel requires native
exit code 134, zero child threads and one execution retirement. This verifies
abort without a POSIX signal dependency. Every process must release its object,
wait, thread, private-memory and reservation charges. Frame counts and transient
kernel-heap usage must return to their measured baselines before the kernel
publishes the trusted RUSTSTD lines. Guest diagnostics cannot supply these lines.

Host checks passed: 45 Python tests, including five source-ownership checks and
32 evidence-parser checks; ten allocator tests; 43 C/C++ header checks; workspace
formatting and Clippy with warnings denied. Independent source review found and
verified fixes for generated-source reuse and unproven mutex contention, then
approved the final source and both completed QEMU records.

## Intermediate failure and build corrections

One earlier one-credit run terminated through the provider's abort fallback
before the mutex-release checkpoint. Its log and evidence are retained as
`wait-credit-*`. The precise abort site was not captured in that run, so this
record does not assign a confirmed root cause from the later successful runs.

The investigation added bounded bridge failure diagnostics, a direct panic
diagnostic hook in the fixture, explicit native abort and proper Cargo dependency
tracking for libc.a/crt1.o/linker.ld. Infallible sleep retries wait-credit
exhaustion against its original deadline. Final runs use the rebuilt provider
and the complete current probe. No error suppression or duplicate-symbol
linker option is used.

## Proof limits

QEMU TCG verifies the selected functionality and fixture retirement. The runner
stops QEMU from the host after completion; this is not guest shutdown or a
hardware performance, energy, durability or recovery measurement.

The Unix extension surface currently covers byte strings and descriptor
ownership. General std filesystem/network/process APIs, real entropy, wall
clock, inherited streams, dynamic TLS/linking and unwinding remain unavailable.
HashMap seeding fails explicitly. BetAFS and ICE were not executed or modified;
their production runtime and storage integration remain separate work.
