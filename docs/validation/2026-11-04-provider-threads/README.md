# Static provider-thread validation

Date: 2026-11-04. Base: 0483bdb; previous static-provider milestone: 256963f.

Four final QEMU TCG cases pass: debug Haswell-noTSX/one CPU with the mixed ABI
probe; release max/four CPUs with mixed ABI and readonly block service; normal
debug max/four CPUs with writable block service; release Haswell-noTSX/one CPU.
All four use the recorded provider-input fingerprint and independently reviewed
probe/package hashes.

Each guest runs three fresh static mlibc processes. Each process creates fourteen
provider C11 threads, one raw detached-thread reaper and two raw allocator
threads: seventeen children and eighteen retired threads including the initial
thread. Required events cover TLS/errno isolation, inherited FP rounding,
mutex/condition/once contention, TSS cleanup, join results, detach and repeated
reuse. TSS values are cleared before destructor callbacks. Publication-failure
and detached-reuse account snapshots balance. Fast self-detach succeeds, but
does not force the precise child-exit/creator-publication interleaving; the
creator-pin lifetime argument is separately reviewed.

The second process exits through the final provider thread and returns zero
after atexit/global destructors. The third has one wait credit and must observe
the native wait-exhaustion fallback while completing the same work. The reaper
skips records still executing language cleanup, then requires actual native
completion before releasing stack, TCB and control ownership. Four trusted
markers appear once only after all three processes retire, with no remaining
objects, waits or account credits and independent logical/physical balance.

The separate completed-wait regression uses zero wait credits. Active targets
still fail admission; completed thread/process observations return full high-bit
codes and separate reasons without a new wait. Expired deadlines, process
self-wait rejection and unbound controls retain their error precedence. A
process result remains observable after VM reaping; result publication itself
continues to promise execution handoff rather than full VM/device cleanup.

Actual-checkout verification matches 1,547 inputs and passes
463 Rust tests, 32 Python tests and 32 C header checks, formatting, normal and
all-feature kernel builds, kernel/allocator Clippy with warnings denied, and
debug/release provider builds. A post-review ADR wording clarification was
synchronized and all final hashes rechecked; executable inputs were unchanged.
Both rebuilds have the same provider-input fingerprint as the guest matrix.
Rebuilt image hashes are recorded independently; this is not a claim of
byte-identical debug artifacts. Provider compile commands retain v3, disabled
MMX and target headers. Independent review checked linked images for zero MMX
and IFUNC entries. Generated-source reuse and rename recovery also pass.

The matrix executes an isolated source snapshot; actual-source rebuilds and
host checks are separately recorded. Earlier failed-build and failed-guest
transcripts remain in the working evidence directory, including
`continuation-work/provider-thread-failures/`; they are not passing matrix cases
and are not copied into this archive. No complete kernel, hosted libc, full
pthread/POSIX implementation, descriptor/terminal service, dynamic TLS, signals,
robust-lock recovery, hardened entropy/startup, C++ library, Rust std, or
bare-metal latency/power result is claimed.

See the source manifest, actual verification, independent review, provider
command audit, four final guest logs/build records and separate native-wait
regression beside this file.
