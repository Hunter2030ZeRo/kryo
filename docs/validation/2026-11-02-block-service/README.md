# Initial block service validation

Date: 2026-11-02.

The final matrix covers 24 QEMU MTTCG boots: default one-CPU guests without an
extra device, discovery-only read-only fixtures, writable block fixtures and
read-only block fixtures; plus max-model four-CPU and max-model two-CPU 129-GiB
writable guests. Every selection runs debug/release and boot-tests/normal.
`evidence.json` and 72 archived guest/stdout/stderr logs record the results.
`source-manifest.json` hashes 201 unchanged source/configuration/runner inputs.
Generated builds, prose and unused ABI drafts are excluded. The unused drafts
were moved outside the source tree after the matrix; every retained input hash
is unchanged.

`host-tests.log` records 391 Rust tests: core 296, HAL 73, interfaces 7, libsys 4
and services 11. Eleven Python evidence tests also pass. `checks.log` records
formatting, normal/all-feature kernel Clippy, interface host Clippy and production
userspace workspace Clippy with warnings denied. The block protocol adds four
host tests for version, instance, gate authority, capability counts, block bounds
and read-only policy.

## Executed service graph

The block service is a separate Ring 3 process in the shared normal runtime,
alongside the supervisor. Startup installs accounts, process-bound Device and
three private gates while its initial thread is suspended. Clients also receive
complete typed startup before Runnable publication.

The first client runs alone after the server reaches its receiving loop. It
submits a read with one Memory capability. The coordinator cancels only after
the server publishes a private DMA request and the client's call is accepted.
The server completes the read, unmaps/closes the loan and tolerates InvalidReply.
The client requires Cancelled with accepted=true, then waits for a successful
Info reply before unmapping or reusing its memory. Driver-bound event 92 and the
client's completion log are required. This establishes ordering without assuming
that a short timeout happens to expire after delivery. Device work may already
have completed by the observed cancellation point.

Read-only and read/write clients then exercise Info, reads, denied write/flush/
stop on the read gate, stale instances, malformed versions, out-of-range blocks,
missing memory rights, repeated two-capability malformed requests and two-cap
fire-and-forget cleanup. A successful final read establishes continued service
after rejection. Gate authority does not depend on a claimed client ID.

The writable client writes block 2, flushes and reads back all 4096 bytes. On a
read-only device the write gate returns ReadOnly without pretending success.
The legacy transport probe still writes block 1. The runner checks the whole
owned scratch image after QEMU stops: [4096,8192) has the old pattern
`((offset * 37 + 11) & 255)`, [8192,12288) has the IPC pattern
`((offset * 17 + 0x53) & 255)`, and every other byte matches the original.
Read-only fixtures remain entirely unchanged. Both driver paths verify the disk
signature before any write. These are disposable file-backed devices, not host
block devices or BetAFS images.

## Persistence and retirement

Normal mode requires all three clients to complete and the live driver to remain
Blocked in ReceiveFor with no pending DMA. The log records this waiting state;
the supervisor remains active too. This proves an initial persistent service,
not general discovery, automatic reassignment or restart.

Boot tests start a separate control client after the data clients finish. It
verifies control-gate Info denial, requests Stop and then exits. The coordinator
waits for driver exit and hardware retirement, reaps all five processes and
requires zero live objects, channel payloads and account usage. Physical frames
restore except measured retained kernel alias-table storage. A Stop reply alone
is not treated as a DMA-retirement acknowledgement.

Failed device cleanup now leaves execution resources retained. Normal idle owns
a retry timer no later than one millisecond or the next existing IPC deadline,
so retry progress does not rely on an unrelated service waking the coordinator.
This branch was source-reviewed against the existing controlled IRQ/idle path;
the matrix does not inject a failing hardware reset into the normal service.
Earlier transport failure-injection regressions continue to run in boot-tests.

Independent code review found no blocker in gate, loan, cancellation, suspended
publication or retirement ordering. It independently ran interface 7/7 and Python
11/11 tests and checked an initial guest log and scratch-image hash. The periodic
three-gate polling limitation remains documented.

## Scope

The graph is opt-in through BLOCK_PROBE=1 and the signed disposable-disk setup.
The service currently rotates three gates with one-millisecond waits; completion
also uses transport polling. No storage IRQ, wait-any, physical power/latency,
IOMMU, malicious-device containment, production device inventory or recovery
claim is made. The 129-GiB guests are demand-backed sparse checks, not a full RAM
sweep. Existing memory, IPC/budget, SMP/TLB, thread/TLS/SIMD and supervision checks
remain part of every applicable run. The runner stops only its emulator after
markers; guest shutdown is not verified.

Native binaries remain static ELF64 AMD64 with the existing bootstrap soft-float
target and startup-handle convention. A dedicated hard-float target, C/Rust FFI
executable proof, crt0/argc/envp/auxv, ELF TLS and native standard-library ports
are not part of these tested sources. There is no VFS or BetAFS mount yet.

## Reproduce

Run `python3 tools/boot_smoke.py --all`, with `--normal` for the normal graph.
Add `--pci-probe` for discovery only, or `--block-probe` for transport plus IPC
service checks. Add `--block-readonly` for read-only policy/device failures.
The SMP selection is `--cpu max --cpus 4`; the large sparse selection is
`--cpu max --cpus 2 --memory 129G`. Each keeps separate evidence and preserves
`build/debug.log`. The existing 90-second boot timeout and 2048-MiB RSS guard
remain unchanged. Host/lint commands are recorded verbatim in their logs.
