# Fallible stdio validation

Date: 2026-11-04. Base: 784f5b5. The fixed eleven-file bundle matches all
1,686 files in the actual checkout and the isolated stdio snapshot.
Endpoint-gate changes in the separate working stage are outside this bundle.

Two final QEMU TCG cases pass: debug max/four CPUs and release Haswell-noTSX/
one CPU. Each runs three fresh libc processes, records event 210=1 three times,
no event 211, and one trusted LIBCSTDIO retirement marker. All existing provider
markers, join/detach/last-thread/wait-credit checks and required events also pass.
The per-process watchdog remains thirty seconds with the same private-memory
quota. These runs enable the libc probe; the mixed ABI probe is disabled.

The native probe commits most of its private account in bounded chunks, drains
existing heap slack, then verifies ENOMEM and stream errors from fputc/ungetc.
After releasing its allocations and VM reservation it checks pushback recovery,
restores errno and FP state, and leaves stdout without dirty data. Forcing buffer
mode avoids descriptor probes; successful pushback recovery performs no file I/O.
The trusted kernel marker follows process, object, account and backing retirement,
with separate logical-heap and physical-backing balance.

FILE storage is acquired before Open, avoiding an opened owner on allocation
failure. Buffer arithmetic and allocation are checked. Read publishes consumed
pushback as a short read before later I/O; fread preserves complete elements
across subsequent errors or EOF. The actual generated fopen, buffer-allocation,
read and fread bodies separately pass 25 host-only scenarios at O0 and O2 under
AddressSanitizer and UndefinedBehaviorSanitizer with allocator/Open/I/O doubles.
Those tests cover construction, cleanup, overflow, partial elements and
pushback before EIO, EOF, reset and write-back failures. LeakSanitizer is disabled
because the host restricts ptrace; explicit allocator counters check the tested
FILE cleanup paths and do not establish general leak freedom. Reverting only
the read body to its previous version makes the regression control fail case 13.

Four earlier guests are retained as failures, with zero contribution to passing
counts. Their kernel watchdog reports Libc probe stalled. Progress instrumentation
shows at least 1,024 successful 4-KiB malloc allocations in the power-counter run;
it does not establish a deadlock or the exact cost of the final growth call.
After replacing repeated heap growth with bounded native commits, both final guests
pass without raising the quota or watchdog.

Actual-source checks pass 476 Rust tests, 34 Python tests, 43 C header/checker
cases, formatting, normal/all-feature builds, warnings-denied Clippy and debug/
release provider rebuilds. Host source scenarios are separate from those counts.
Both actual rebuilds and both guest packages share the provider fingerprint,
six probe sources, allocator inputs and all nine patch hashes. Five cached
dependency archives match their pinned hashes and sizes. Packaged ELF hashes
and the exact host-tested generated function bodies are checked.

Independent read-only review found no source blocker. The executor separately
checked final guests and actual artifacts; independent-review.json preserves
that distinction. Logs retain their original bytes. No successful native fopen,
file service, stdout transport, full hosted standard library or bare-metal
performance result is claimed.
