# Static TLS and mapped ELF metadata

Date: 2026-11-03.

Six final QEMU TCG cases passed: debug/release Haswell-noTSX boot tests on one
CPU with the TLS probe; debug/release normal boots on four max CPUs with the
probe and writable/read-only block fixtures; and two four-CPU legacy regressions
without the optional probe (debug boot tests and release normal boot).

Enabled runs require actual mapped ELF/PHDR metadata, compiler local-exec TLS
in C/C++/Rust, initialized data and zero-fill, 64-byte alignment, main/two-child
isolation across yield/sleep/join, and native execution-resource retirement.
The existing mixed-language ABI and live-vector preemption checks also pass.
Post-join clearing is POD fixture cleanup, not language destructor support.

The four probe images retain compiler/target/lock/image hashes and program-header
metadata. Debug uses 13 LOAD pages and release 12, within the unchanged 16-page
limit. The template is read-only with filesz 136, memsz 240 and alignment 64.
Kernel staging copies match their private build artifacts. The two complete
scratch-disk images match expected hashes. Legacy images omit the probe module.

Host tests passed: core 314, HAL 77, interfaces 7, libsys 4 and services 11
(413 total). ELF adds 11 tests for metadata, TLS and malformed descriptors.
Seventeen Python tests cover marker admission and the mapped-header packaging
comparison. Kernel normal/all-feature, scoped core ELF/stack, HAL and custom
local-exec user-target Clippy checks passed with warnings denied; Rustfmt passed.
The scoped host commands and unchanged source hashes are in host-validation.json.

The first trial reached main TLS initialization but failed with code 225 when
creating its second child. The probe still used the old bounded two-thread
fixture constructor. It now uses growing production task/object/wait registries;
the three-thread account is an explicit test budget. The failed debug log and
evidence are retained as prior-inline-fixture-* and excluded from the six final
passes. No global process/thread limit was introduced.

source-manifest.json records 280 active source/build inputs. The exact host and
guest logs are adjacent. Packaging preserves runtime load bytes except the
explicit ELF64 section-table bookkeeping fields documented in ADR 0025; the
actual output retains objcopy's valid metadata.

This proves a bounded static TLS runtime fixture under QEMU. It does not provide
the full mlibc TCB/DTV, dynamic TLS, TLS destructors, a general private allocator,
pthread conformance, physical hardware performance or energy results. Existing
kernel scratch-TLS mappings remain unused by the fixture. Host termination of
the owned emulator is not proof of guest shutdown.
