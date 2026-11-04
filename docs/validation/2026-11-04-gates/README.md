# Endpoint-gate validation

Date: 2026-11-04. Base: 5248cf2. The fixed 38-file source bundle matches
all 1,740 files in the actual checkout and staged copy.

Four final QEMU TCG guests pass with mixed ABI and static libc probes: debug
Haswell-noTSX/one CPU; release max/four CPUs with readonly block service;
normal debug max/four CPUs with writable block service; release Haswell-noTSX/
one CPU. Each has one CGATE event 102=1 and trusted retirement marker, plus the
existing CIPC/event 100=1 and CREPLY/event 101=2. Each runs three fresh libc
processes, including stdio recovery event 210=1, provider thread/TSS cleanup,
last-thread exit and wait-credit recovery. All required markers and events pass.

The C/Rust fixture creates two immutable, full-width gate labels for one receiving
thread. It checks a duplicated SEND authority on Request, then a gate returned
through ReplyCaps on STOP; both retain label A. Last-reference closure produces
the zero-payload GateClosed event with no token or grants. A retained second
gate observes receiver death and keeps its charge until final release. The C
wrapper separately checks minting on its own endpoint and closed-root behavior.
Kernel teardown checks process, channel, object, quota, heap and backing retirement.

The three boot-test guests also require the gate lifetime/layout, three loan,
process-retirement wake and legacy-exchange register markers. These fixtures
are not compiled into the normal guest; its real C/Rust gate check still runs.
Three native cases execute Ring 3 Send/Call while a trusted fixture closes the
blocked caller's source handle to model sibling closure. The fourth executes
Ring 3 Exit and actual process reaping, then checks the blocked receiver becomes
runnable before any polling, deadline or extra settlement step. These different
interleavings are not interchangeable proofs. The existing six-case ReplyCaps
regression remains enabled, including a raw post-response RCX/R15 preservation
check for previously compiled CallExchange callers.

Only Receive/ReceiveUntil/ReceiveFor decode kernel label/event outputs. Other
SDK results normalize them to zero/Message without interpreting preserved input
registers. GateClosed reports authority retirement; it does not prove accepted
service-job completion or cancellation acknowledgement.

On the pinned x86-64 build, Endpoint<2,2> stays 312 bytes and KernelObject stays
328 bytes. Completion grows from 72 to 80 bytes; GateRecord is 72 bytes and gate
payload fields total 24 bytes. Type-size logs record this layout, including the
existing enum padding. A changed or saturated final-reference epoch drives
collection, while unchanged steady IPC skips registry scanning. Mint and
last-reference collection remain O(gates). No throughput or latency improvement
is inferred from layout or source structure.

Actual-source checks pass 486 Rust tests, 35 Python tests, 43 C checker
cases, 248 profile constants and 106 native constants, formatting, normal/all-
feature builds, warnings-denied Clippy and two provider rebuilds. Eight C/C++
O0/O2, PIC/frame-pointer IPC builds have no undefined helper symbols; guarded
host paths run only after excluding native int80. This does not mean every
wrapper was guest-executed. Kernel-only preintegration logs separately cover
456 host and nine libsys tests and one QEMU guest; their 26 code hashes match
the final sources. These counts are not added to the combined results.

Actual rebuilds and all four packages share the provider fingerprint, six probe
sources, allocator inputs, nine patches and five verified dependency archives.
ABI/libc ELF and peer package-copy hashes are checked. Independent read-only
source review and executor verification of the final matrix/actual artifacts
remain distinct in independent-review.json. Raw log bytes are preserved.
No file service, BetAFS integration, standard FD implementation, complete
production kernel or hardware performance result is claimed.
