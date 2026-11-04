# Capability-bearing reply validation

Date: 2026-11-04. Base: 68fd161. The fixed 22-file source bundle matches
all 1,627 expected files in the actual and staged copies.

Four final QEMU TCG cases pass with both mixed ABI and static libc probes:
debug Haswell-noTSX/one CPU; release max/four CPUs with readonly block service;
normal debug max/four CPUs with writable block service; release Haswell-noTSX/
one CPU. Each records one CIPC marker/event 100=1 and one CREPLY marker/event
101=2. The six-case REPLYCAPS mechanism marker appears once in each of the three
boot-test guests; that fixture is not compiled into the normal guest.

CallExchange reserves reply handle capacity before publishing a request.
ReplyCaps stages reduced-rights references before consuming a live single-use
token, then publishes message and grants together. Retry after invalid rights,
source or capacity does not partially publish a reply. Quota failure, timeout,
cancellation and retirement release reservations and retained references.
An accepted timeout does not establish cancellation of device work or buffer loans.

The C client delegates one READ|MAP memory page to a separate Rust process.
The peer rejects writable mapping, checks all 512 u64 words, unmaps and closes
the request grant before responding. Its response atomically returns a separate
READ|MAP memory object, a SEND-only endpoint and the four-word message. The C
client checks actual rights, denied write/escalation, all 512 response words and
then closes the response owners. Read-only grants do not freeze other aliases.
STOP, native process observation and endpoint owner death follow. The ABI
application and peer retire fourteen threads; the application's thirteen-thread
WAIT regression remains unchanged. Channel, object, wait and account retirement
checks retain separate logical-heap and physical-backing balance.

The existing provider regression still runs three fresh libc processes per
guest, covering join/detach, TSS cleanup, last-thread exit and one-wait-credit
recovery. Its markers and event counts remain unchanged.

Actual-source checks pass 476 Rust tests, 34 Python tests and 43 C header/checker
cases, formatting, normal/all-feature builds, warnings-denied Clippy and both
provider rebuilds. Eight C/C++ O0/O2, PIC/frame-pointer IPC builds have no
undefined helper symbols. Three guarded host checks build, inspect and execute
rejected-input paths after excluding native int80. This compile/host coverage
does not mean every wrapper was guest-executed.

The earlier mechanism snapshot separately passed 466 Rust tests, 32 Python tests
and two standalone QEMU guests. Its ten mechanism code files match the final
sources; core-evidence.json preserves commands, results and hashes. These counts
are not added to the final combined results. The initial marker-spelling guest
failed because the checker expected different wording despite the actual kernel
marker being present. Its evidence is retained as a failure and excluded from
passing counts.

Provider rebuilds and all four final guests share the recorded input fingerprint,
probe and allocator inputs. ABI/libc ELF hashes, package copies and the Rust
peer's package copy are checked. Independent read-only review found no source
blocker; final matrix and actual-artifact checks were performed separately by
the executor, as recorded in independent-review.json. Raw log bytes are preserved.
The combined matrix runs from an isolated source snapshot; actual-source builds
are separate evidence. No complete kernel, FD service protocol, VFS/stdio/PTY,
full hosted standard library or bare-metal performance result is claimed.
