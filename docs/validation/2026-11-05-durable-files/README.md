# Durable file-service validation

Work: 2026-11-05–06. Baseline: 9625456e3290748f6588b0c7af3687bb79d3ebaf.

The native file service now provides directory iteration and advisory shared/
exclusive locks. A separate Ring 3 BetAFS adapter connects the actual engine
to block IPC and real FLUSH barriers. Both sync_data and sync_all invoke the
engine filesystem barrier; directory handles can synchronize too. Ordinary
sync adds no validation debug syscalls. See [ADR 0048](../../decisions/0048-directories-and-file-locks.md)
and [ADR 0049](../../decisions/0049-native-betafs-durability.md).

## Guest results

| Configuration | First boot | Second boot | Result |
| --- | --- | --- | --- |
| Debug, Haswell-noTSX, one CPU, boot tests | Explicit format/write/sync | Open existing same disk | Passed |
| Release, max CPU, four CPUs, normal supervisor | Explicit format/write/sync | Open existing same disk | Passed |

The harness creates one owned 32-MiB image per configuration. The filesystem
uses physical blocks 1 onward; all 4096 bytes of block zero remain unchanged.
The first guest writes deterministic 12-KiB contents, exercises positioned I/O,
resize/zeroing, actual metadata/directories and three explicit data/all/parent
barriers. After the host kills QEMU, a separate guest opens the same file
without recreation or formatting and checks content and metadata. The final
post-write/post-open disk hash is identical in both configurations:
e822886c49d2401e1757f2d92f6a1e51e6e520e766f4c465923762abf189ee1a.

Each guest executes actual Raw, Zstd, LZ4 and OpenZL encoding/decoding, checks
that the selected codec matches the request and reopens engine files stored
with all four codecs. This codec-reopen subtest uses an explicit RAM device;
the separate disk-backed file-service test uses the engine default compression.

For every traced sync, the kernel snapshots the hardware completion count at
the `312` entry and requires a new successful acknowledged VirtIO FLUSH before the `311` completion.
Prior commit barriers cannot satisfy a later no-op sync. All four barrier
pairs, including final service sync, passed in every guest.

Separate clients test lock-owner death. The waiter must be Blocked in
CallExchange with its wait charged before the owner is stopped. The kernel
observes Stopped with code 91 after execution completion, reaps the owner and requires
the waiter to acquire/unlock successfully. Both services, all clients, wait
registrations, accounts, gates, DMA transport and backing retire under trusted
checks. Final debug boot tests also complete their global teardown; release
continues into the normal supervisor.

The final standalone ABI/libc/std debug run also passed directory snapshots,
independent cursors, EOF/error handling, alias lock lifetime and an actually
blocked lock child, together with the previous runtime regressions.

## Host checks and source integrity

- The actual C provider passed 106 behavioral groups at O0 and O2 under ASan
  and UBSan. Malformed entries/replies, unchanged outputs, descriptor lifetime,
  accepted ambiguity and lock error mapping are covered.
- Interface tests: 18. Actual volatile protocol/backend harness: 29. Actual
  BetAFS frontend/engine tests: 16. Existing engine/wire contract tests: 15.
  Storage faults cover uncertain writes/barriers, queue draining and retained
  orphan ownership after failed final-unpin. Untraced sync success/failure is
  exercised directly against the engine.
- All 52 Python tests, formatting, workspace Clippy and native C++ syntax checks
  passed. A separate reviewer reran provider/frontend tests and inspected all
  final guest logs and source/image hashes; no findings remain.
- Builds verify locked registry archives and source trees, including original
  libc/OpenZL packages before owned-copy patching. Only OpenZL cross-build
  recipes change; algorithms and the independent dirty BetAFS tree stay intact.
  Output/source overlap and symlinks are rejected before publication. No MMX
  appears in the packaged native programs.

Reproduction commands and prerequisites are in the [adapter README](../../../userspace/filesystems/betafs/README.md).
Use distinct owned output directories and pinned offline source caches. Existing
disk/log outputs are preserved; selecting an existing volume for formatting is
not an automatic recovery action.

## Retained failures and proof boundary

The first build omitted the source-cache selection. Initial actual guest
attempts then exposed the old one-MiB module bound and two trusted-harness
termination-observation mistakes. The corrected loader admits larger programs
explicitly; stop requests are followed by execution completion and terminal
observation before reaping. Initial logs remain as initial-* files; their
original disk images remain outside Git. Successful results are the final runs.

Source/image records accompany each run. Guarded application verified all
102 reviewed files in the actual repository, followed by formatting and diff
checks; actual-verification.json records those checks. Native programs were
tested in source-matched isolated builds and were not rebuilt or executed
again after application. Archived log copies trim trailing whitespace;
original streams and binaries remain in the external work directory.

These are QEMU TCG and actual VirtIO FLUSH/reopen results. Host SIGKILL is not
guest shutdown or physical power-loss evidence. Hardware durability/performance,
general device/mount policy, namespace mutation, engine symlinks, timestamps,
libc++/pthread and complete hosted POSIX/std remain separate work. No independent
BetAFS or ICE source was changed.
