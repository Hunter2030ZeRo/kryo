# x86-64-v3 hardware admission

Date: 2026-11-02.

Five positive QEMU TCG runs passed: debug/release Haswell-noTSX on one CPU,
debug max on four CPUs with boot tests and a writable block fixture, and
debug/release normal service boots on four max CPUs with writable/read-only
block fixtures respectively. Each run required one admission marker per CPU;
boot tests additionally required successful XSAVE/AVX state preservation.
All three attached scratch-disk hashes matched their expected complete images.

Three negative runs used the release test ISO: qemu64, max without AVX2 and max
without FMA. Each produced the expected early rejection and never published
CPU admission, frame readiness or a supervisor. The host terminated each owned
emulator after observing the result; these tests do not establish guest shutdown.

Host tests passed: core 296, HAL 77, interfaces 7, libsys 4 and services 11
(395 total). HAL adds four feature/XSAVE admission tests, including missing
leaves, every required bit, bootloader OSXSAVE state and bounded YMM storage.
The Python evidence parser passed 12 tests. Kernel all-feature and HAL host
Clippy with warnings denied, plus Rustfmt, passed.

`evidence.json` retains run settings, commands, checks and original artifact
locations. Copied debug/stdout/stderr logs and rejection logs are adjacent.
`source-manifest.json` records active non-documentation source/build inputs.

The change enforces hardware admission, not a newly built native language
sysroot. Kernel and bootstrap Rust compilation remain soft-float. No libc was
built or executed, no Intel GPU was initialized, and no physical performance,
power or Vulkan conformance result is claimed.
