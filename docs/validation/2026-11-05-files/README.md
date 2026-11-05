# Native FILE service validation

Date: 2026-11-05. Base: `a3aec0e45e842a67a38a5f24929ced5acd209f25`.

The static ANSI mlibc profile now runs real FILE operations against a separate
Rust Ring 3 service through authenticated endpoint gates. The provider owns
indexed descriptors and operation pins; the service owns access modes, append
selection and file position. No new privileged filesystem policy or syscall
was added. [ADR 0040](../../decisions/0040-service-backed-files.md) defines the
experimental protocol and lifetime rules.

## Results

| Scope | Result |
| --- | --- |
| Rust host tests | 491 passed |
| Python tool/protocol checks | 37 passed |
| C/C++ header/compiler checks | 43 passed |
| Verbatim file runtime with host native-call doubles | 30 cases at O0 and O2, ASan/UBSan |
| Pinned stdio function bodies with host I/O doubles | 25 cases at O0 and O2, ASan/UBSan |
| Normal/all-feature kernel builds, format and Clippy | Passed; Clippy warnings denied |
| Actual static provider builds | Debug and release passed |
| QEMU TCG integrated boots | Four final configurations passed |

The final matrix covers debug/Haswell-noTSX/one CPU,
release/max/four CPUs with read-only block storage,
debug normal boot/max/four CPUs with writable block storage, and
release/Haswell-noTSX/one CPU. Every guest runs the ABI probe and three fresh
libc process instances. The normal-boot case also reaches the existing
persistent supervisor. Existing gate, ReplyCaps, thread, TLS, SIMD and applicable
block regressions remain required by the parser. Temporary block disks contain
only the existing signed block fixture; no filesystem image was mounted.

Actual C fopen/fwrite/fflush/ftell/fseek/fread/fclose cross address spaces to the
Rust peer. Tests include a 9000-byte payload, short reads and EOF, append after
seek, independent open positions, truncation, read-only rejection, short writes
and ENOSPC. Raw gate tests cover shared position through duplication, namespace
versus file authority, loan bounds and insufficient rights. One internal
native descriptor is intentionally left for actual process-handle retirement.
The peer waits without a polling timeout and exits only after final namespace
and file authority closure. Kernel success requires both process exits and
complete object, wait, channel-account, heap and frame retirement.

## Failure and concurrency evidence

The host file harness compiles the verbatim backend and native acceptance
helper, replacing only allocation/native-call boundaries. Cases cover table
allocation/growth failure, preservation of live entries, numerical reuse while
an old operation is pinned, malformed replies, capability cleanup, mapping-slot
reuse, mutex lost-wakeup cutpoints and wait-credit fallback. Descriptor lookup
is indexed; final-reference unlink remains a cold list scan. No hardware
latency or throughput result is implied.

Review found and corrected an Open publication use-after-free. A deterministic
close at publication exposed the stale dereference under ASan; restoring the
old ordering reproduces the fault. Review also confirmed that the provider can
retain dirty bytes after an accepted Write cancellation and flush them again.
The backend now preserves the first error and disables further I/O on that
description. The regression verifies no second RPC after accepted cancellation,
while undelivered cancellation stays retryable. Removing only this guard makes
the regression fail. These are host cutpoints combined with the pinned
provider/core source paths recorded in review.json, not a new native FILE
cancellation scenario. Close remains usable for retiring the descriptor.

The initial guest failed at stage757 because a fixture inspected capability
fields that legacy CallCaps does not define. The corrected fixture uses
CallExchange with reply capacity zero. No SDK contract change was needed.
The failed guest log, metadata and evidence are retained and excluded from
passing results. Intermediate successful snapshots before indexed lookup and
accepted-failure protection remain outside this final evidence set.

ASan and UBSan are enabled in the host harnesses. LeakSanitizer is disabled
because of the host execution constraint; explicit owner counters check file
records, descriptor tables, memory capabilities and mappings. Host Open/Read
and transport doubles do not establish native filesystem support.

## Reproducibility and boundary

results.json records the final source bundle, guest metadata, packaged module
hashes and archived log hashes. source-manifest.json records source inputs.
Each provider build checks pinned archives, generated sources, seven probe
inputs, the target and allocator metadata. The packaged ELF and Rust peer
copies are hash-checked, and every generated files.cpp matches the actual
checkout. All final guests and actual provider builds use the same source
fingerprint. Rust/kernel/Python/header checks are reused only across the final
C++-backend/harness-only corrections, with their inputs byte-identical; the
changed backend and provider artifacts were rebuilt and tested again.
Final documentation changes do not alter executable inputs.

This backend is a bounded volatile validation service with a 16-KiB file and
eight open-description slots. Those bounds are fixture policy. The generic
provider descriptor table grows fallibly and has no eight-file limit. The
native transfer bound is one page per RPC; mlibc loops for larger FILE work.
Append is atomic per RPC, not across a complete multi-RPC fwrite.

Production namespaces, mounted BetAFS, persistent/durable storage, inherited
standard streams, public POSIX descriptor functions, readiness, terminals/PTY,
pthreads and complete C++/Rust standard libraries remain separate work.
fflush only delivers buffered bytes; it does not establish durable storage.
