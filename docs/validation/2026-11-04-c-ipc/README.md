# C native service-call validation

Date: 2026-11-04. Base: e04e99e. The bounded source bundle is fixed in
evidence.json; actual and staged copies match all 1,586 expected files.

Four final QEMU TCG cases pass with both mixed ABI and static libc probes:
debug Haswell-noTSX/one CPU; release max/four CPUs with readonly block service;
normal debug max/four CPUs with writable block service; release Haswell-noTSX/
one CPU. Each has one C IPC event 100=1 and one trusted CIPC retirement marker.

The C client receives a SEND-only endpoint from a separate Rust service and
checks denied TRANSFER before acceptance. It grants READ|MAP access to one
shared page. The Rust peer rejects writable mapping, verifies all 512 u64 words,
then unmaps and closes before replying. The sender keeps the page unchanged
through this interval; a read-only grant does not make its other aliases immutable.
STOP, native process observation and endpoint owner death follow. The kernel
requires fourteen retired threads across the ABI application and peer; the
application's thirteen-thread WAIT regression remains unchanged. Channels,
objects, waits and account charges retire, with separate logical-heap and
physical-backing balance.

The existing provider regression still runs three fresh libc processes per
guest, including detach/reaping, TSS cleanup, last-thread exit and one-wait-credit
recovery. Its markers and required events remain unchanged.

Actual-checkout checks pass 463 Rust tests, 33 Python tests and 43 C header/checker
cases, formatting, normal/all-feature builds, warnings-denied Clippy and both
provider rebuilds. The C checker includes eight C/C++ O0/O2, PIC/frame-pointer
builds without undefined helper symbols. Three guarded host checks build,
inspect and execute rejected-input paths only after excluding native int80.
This compile/host coverage does not mean every wrapper was guest-executed.

Provider rebuilds and all four guests share the recorded provider-input
fingerprint, probe and allocator inputs. Packaged ABI/libc ELF hashes and peer
package-copy hashes are checked. Independent read-only review found no source
blocker; the executor separately checked the final matrix and actual-source
artifacts. The review record preserves that distinction.

The matrix uses an isolated source snapshot; actual-source rebuilds are separate
evidence. Two earlier provider build logs were restored byte-for-byte from their
retained originals so their recorded hashes match; executable inputs did not
change. This archive preserves log line endings. The initial failed guest
incorrectly treated CallCaps raw count/handle
fields as Receive results. The condition was corrected and that transcript
remains under `continuation-work/c-ipc-failures/call-result-fields`; it is not a
passing case. Reply-capability extensions under separate development are outside
this bundle. No complete kernel, VFS/stdio/PTY, full hosted standard library,
hardened startup, or bare-metal latency/power result is claimed.
