# Implementation status

Snapshot: 2026-11-05. The [design contract](design-contract.md) defines requirements; this file records implemented behavior.

[Namespace metadata lookup](decisions/0047-path-metadata.md) now connects
Rust metadata, symlink_metadata and existence checks to a dedicated Ring 3
request. It uses explicit final-symlink policy, preserves byte paths and
propagates permission, loop and resource errors. No file descriptor or open
gate is created for a lookup. The bounded volatile fixture exercises directories,
links, dangling targets and cycles; it does not implement a general namespace.
[Validation](validation/2026-11-05-path-metadata/README.md) covers native
execution, concurrent lookups, failure recovery and complete retirement.

[Descriptor metadata and positioned I/O](decisions/0046-descriptor-metadata-and-positioned-io.md)
now add genuine length/type/readonly snapshots, cursor-preserving At operations
and checked resize through the Ring 3 file service. File Sync All/Data requests
are forwarded explicitly; the volatile backend rejects both instead of claiming
durability. POSIX WriteAt behavior honors its offset on append
descriptions. [Validation](validation/2026-11-05-file-ops/README.md) covers
concurrent aliases, zero-filled growth and real wait-credit rejection/recovery.
Directory iteration, timestamps, permission mutation and a persistent filesystem
backend remain separate work.

The [optional CPU random source](decisions/0045-cpu-random-bootstrap.md) supplies
two register words per native call through bounded BSP RDRAND sampling. The
initial policy trusts that CPU source alone; it does not raise v3 requirements.
Unavailable/busy results publish no random words, and a fallible private C
bridge commits bounded buffers only after success. Std RandomState/HashMap now
uses this path on supported hardware. Public getentropy/getrandom, startup
hardening, independent suppliers and hardware entropy assessment remain pending.
See the [validation record](validation/2026-11-05-cpu-random/README.md).

The [native std file profile](decisions/0044-native-std-files.md) connects File,
OpenOptions, Read/Write/Seek, descriptor ownership and shared-cursor clones to
the existing Ring 3 file-service contract. Multi-page reads/writes and final
capability closure are covered in the [file validation record](validation/2026-11-05-std-files/README.md).
Descriptor metadata, positioned I/O and length mutation now use the extension
above. Durable storage and mounted BetAFS remain separate work.

The [restricted Rust std profile](../userspace/rust-std/README.md) builds the
pinned library sources with native allocation, threads, futex synchronization,
TLS destructors and monotonic time. It uses a separate experimental Unix-family
target and one shared C/Rust Talc heap. The [implementation decision](decisions/0043-restricted-native-rust-std.md)
records the source and lifecycle boundaries. Full Unix extensions, directory iteration, durable file synchronization,
independent entropy suppliers, wall clock, hosted ABI and upstream target support remain pending.
The bootstrap target and existing freestanding services are unchanged. See the
[native validation record](validation/2026-11-05-rust-std/README.md).

[Descriptor duplication](decisions/0042-descriptor-duplication.md) extends the
static provider with shared open-description ownership, descriptor-local flags
and atomic replacement. The [validation record](validation/2026-11-05-descriptors/README.md)
covers real C/Rust service execution and host failure cutpoints. POSIX as a whole
remains disabled; inherited streams, exec and complete Rust std remain pending.
The [Rust std platform direction](decisions/0041-cryolite-rust-std.md) now selects
Unix APIs with Cryolite-specific implementations. The bootstrap target cfg and
std=false metadata have not been changed.

[BetAFS boundary verification](validation/2026-11-05-betafs-boundary/README.md)
checks the actual independent engine against native block/file contracts using
an isolated host harness. Separate QEMU tests cover the current Ring 3 block
service. This does not execute BetAFS in Kryo. Native Rust/std and codec support,
host FileDevice portability and general block-device admission remain gates;
the scratch-disk marker restriction is preserved. No filesystem engine was
copied into Ring 0 or changed in the independent repository.

[Service-backed FILE hooks](decisions/0040-service-backed-files.md) connect the
static ANSI mlibc profile to a separate Ring 3 memory-backed validation peer.
The service owns offsets and append status; local descriptor pins protect
in-flight work across close and numerical reuse. Descriptor lookup is indexed;
accepted IPC failures disable further I/O to prevent buffered-write replay.
The [validation record](validation/2026-11-05-files/README.md) separates real
C/Rust guest execution from host fault injection. Production namespace routing,
mounted BetAFS, inherited streams and public POSIX file functions remain pending.

[Endpoint gates](decisions/0038-endpoint-gates.md) route distinct immutable
authorities through one receiver and report final-authority closure. Event
storage is reserved at mint; queued operations and grants retain gate ownership.
Final process-handle destruction settles before idle, so closure can wake an
unbounded receiver without polling. Closed-root stale handles retain their
charge. C/Rust SDK routing and retirement are covered in the
[validation record](validation/2026-11-04-gates/README.md). The file follow-up adds a bounded synchronous protocol; asynchronous
server-side accepted-operation ownership remains separate work.

The [stdio correction](decisions/0039-fallible-provider-stdio.md) makes FILE and
buffer allocation fallible and preserves completed fread elements on error.
A real quota-exhaustion probe checks buffer failure and recovery; a source-body
host harness checks Open/Read failures separately. The file follow-up supplies a validation service backend; production
namespace and storage integration remain pending. See the
[validation record](validation/2026-11-04-stdio/README.md).

[Capability-bearing replies](decisions/0037-capability-bearing-replies.md)
add explicit caller reply capacity and atomic message/grant delivery while
preserving existing Call/Reply. Preflight failures preserve a live token for
retry, and caller handle reservations are committed before wake or retirement.
Rust and C wrappers expose the extension. See the
[validation record](validation/2026-11-04-reply-caps/README.md). File service
objects and descriptor lifetimes remain a separate contract.

The [C service-call SDK](decisions/0036-c-service-calls.md) exposes existing
IPC, capability, notification and shared-memory calls without changing native
call numbers. The ABI fixture connects C to a Rust peer in another address
space and checks a reduced read-only memory loan, acknowledged cleanup and
peer death. See the [validation record](validation/2026-11-04-c-ipc/README.md).
This is the transport foundation; file/terminal services remain separate work.

Provider C11 threads now create dedicated TCB/static-TLS storage, inherit the
creator's FP environment and support join/detach through actual native
completion. A raw process-local reaper retires detached children; creator pins
protect publication. Completed native thread/process waits bypass wait credits.
The [thread contract](decisions/0035-provider-thread-lifetime.md) records the
limited static profile and remaining initial-thread/POSIX boundaries. The
[validation record](validation/2026-11-04-provider-threads/README.md) covers
three process instances per guest, including wait-credit pressure and the
last provider thread's exit callbacks.

The [static mlibc milestone](validation/2026-11-04-static-mlibc/README.md)
builds and executes pinned upstream mlibc with Cryolite sysdeps and a fallible
Talc user heap. Two fresh C process instances exercise startup, real provider
TCB/static TLS, errno, allocation failures, formatting, guarded-page strings,
sleep and exit callbacks. Separate raw native threads test the allocator only.
File calls still fail explicitly; the provider-thread follow-up extends this
baseline. No entropy, hardened startup, pthreads, general stdio/VFS, C++ standard
library or Rust std is implied.
[ADR 0034](decisions/0034-static-mlibc.md) records the supported boundary.

The [process-link milestone](validation/2026-11-04-process-links/README.md)
replaces nested process/member scans on image-runtime resource lookup with
generation-checked thread backlinks. Membership remains canonical; cleanup
failure retains the link and successful retirement clears it. Boot checks
cover growth, rollback, cleanup retry and identity reuse. BSP serialization
and scheduler registry scans remain. No hardware latency measurement is claimed.

The [packed heap milestone](validation/2026-11-04-heap/README.md) reuses Talc
5.1.1 over the existing owned-page backing. Small objects share arenas; public
allocation failure, layout admission and IRQ-before-lock ordering remain
explicit. Physical backing and logical live-object accounting are checked
separately. A boot-only all-CPU probe validates concurrent allocation and full
retirement; kernel policy is still BSP-serialized. [ADR 0032](decisions/0032-packed-kernel-heap.md)
records the integration and native-toolchain revalidation.

The [native C thread milestone](validation/2026-11-04-c-threads/README.md) adds
C creation, exit, join and timed join over the existing kernel calls. Its Ring 3
fixture checks raw entry, private stack/TLS lifetime, high-bit exit codes and
completion before backing reuse. [ADR 0031](decisions/0031-native-c-threads.md)
records the boundary. These calls do not implement pthreads or a libc TCB.

The [security policy](../SECURITY.md) and [kernel SOF contract](decisions/0030-kernel-sof.md)
record kernel-only ownership and two candidate-verification gates. Download
verification failure rejects the update. Verification or technical failure
during an authorized kernel SOF attempt requires reboot and blocks further
attempts until reboot. These are requirements; no kernel verifier, reboot-state
enforcement or live kernel replacement is implemented. Ring 3 service SOF
belongs to a separate user-space manager.

The [native C milestone](validation/2026-11-04-native-c/README.md) adds
freestanding C call wrappers, experimental public C profile source headers and
DebugWrite (62). The optional Ring 3 C fixture directly exercises native memory,
clock, waits and diagnostics, including capability and bad-range failures.
The bounded cold diagnostic path snapshots owned RAM before escaped output;
it does not provide stdio or VFS streams. [ADR 0029](decisions/0029-native-c-and-diagnostics.md)
records the mechanism and concrete pinned-provider integration gates.

The [C ABI registry milestone](validation/2026-11-04-c-abi-registry/README.md)
disables native MMX code generation in both the Rust target and C/C++ probe.
Four QEMU TCG ABI runs pass at the Haswell v3 boundary and with four CPUs;
linked probe disassembly contains no MMX instructions. The platform contract
prioritizes performance and limits x87 to the selected long-double/environment
boundary. This is functional and code-generation evidence, not hardware timing.

Owned errno, memory/time, file and signal proposals now give concrete numbers
and layouts. Eleven compiler checks cover candidate C/C++ target and Rust host
layouts, table consistency, v3 instructions and rejected profile changes.
The compact 64-position signal set does not implement signal delivery.
Installed headers, resource/network/terminal types, provider objects and the
complete hosted libc integration remain separate gates; the later static
mlibc milestone implements the limited initial provider build.

The [Cryolite C ABI proposal](decisions/0028-cryolite-owned-c-abi.md) records the
decision to own public definitions before the libc port. Its foundation layouts
are proposals, not installed headers or an ABI release. Pinned provider research
and target layout checks do not establish a built or executed libc.

The [runtime-owned thread milestone](validation/2026-11-03-native-threads/README.md)
adds native call 61 with caller-prepared stack/TLS and optional guard ranges.
Execution loans preserve mappings until actual user termination, then release
before native join observation. User memory remains owned by the runtime after
exit and kernel reaping. The legacy managed call remains available.

The [atomic wait milestone](validation/2026-11-03-atomic-waits/README.md) adds
32-bit process-private Park/ParkUntil/Wake with bounded FIFO wake, absolute
deadlines and mapping-lifetime cancellation. It uses the existing wait table,
quota and scheduler handoff. Shared waits, priority inheritance, robust locks
and a complete pthread/libc integration remain unimplemented.

The [private VM milestone](validation/2026-11-03-private-vm/README.md) adds
reserve/commit/decommit/release to the native ABI. It separates reserved virtual
pages from zeroed committed RAM, retains quota through failed retirement and
verifies process-exit cleanup. The later static mlibc milestone supplies the
first fallible user allocator over this backing.

The [static TLS milestone](validation/2026-11-03-static-tls/README.md) extends
native startup with mapped ELF/PHDR metadata and C/C++/Rust local-exec TLS.
Its runtime-owned fixture verifies main/two-child isolation and full kernel
resource retirement. It does not implement a complete mlibc TCB/DTV, dynamic
TLS, destructors or a user allocator. [ADR 0026](decisions/0026-private-vm-and-atomic-waits.md)
defines explicit private VM and process-private atomic waits.

The [native ABI milestone](validation/2026-11-02-native-abi/README.md) adds a
dedicated v3 hard-float target, an allocation-free initial-stack builder and an
opt-in mixed C/C++/Rust executable. It records six QEMU cases, 402 Rust host
tests and 14 Python tests. The probe is freestanding; mlibc and other language
standard libraries are not implemented.

The [v3 admission milestone](validation/2026-11-02-v3/README.md) records five
positive and three negative QEMU cases, 395 Rust host tests and 12 Python tests.
[ADR 0022](decisions/0022-platform-baseline-and-performance.md) fixes the CPU and
Intel Vulkan requirements; [ADR 0023](decisions/0023-libc-port-evaluation.md)
evaluates libc candidates and the remaining native runtime boundary. No native
libc/std or GPU support is implied by those requirements.

## Memory and boot

- Limine memory/HHDM/image/framebuffer requests, four-level paging, and a 512 KiB debug / 256 KiB release Limine entry stacks.
- GDT/TSS/IST and IDT setup, emergency diagnostics, breakpoint return, debug output, and a 32bpp framebuffer writer.
- Checked physical ranges, reservation subtraction, and a one-time bitmap frame pool with allocation tokens. Bitmap and summary storage is sized from Usable RAM and reserved before pool/heap activation.
- HHDM access, zeroing, and BSP scratch mappings.
- User spaces with growing mapping/table metadata owning their root, intermediate tables, and data frames.
- RO/RW/RX mappings, initialization before publication, inactive-space mutation, and explicit cleanup.
- Private reserve/commit/decommit/release with separate virtual/data-page charges. Commit supplies zeroed RW/NX backing; retirement retains failed owners and charges. Intermediate user tables remain until address-space close.
- Supervisor-only kernel mappings, HHDM NX policy, and read-only aliases for executable/read-only user frames.
- Huge HHDM splitting with retained kernel table ownership.
- A supervisor-only kernel region for uncached APIC access and guarded per-task entry stacks; added leaf tables extend it up to a one-GiB address budget.
- A fallible Talc object heap retaining owned contiguous frame runs behind shared arenas. Empty secondary arenas return backing; the first metadata arena remains retained. Physical fragmentation and one global heap mutex remain limitations.

Core remains HAL-independent through the unsafe VmBackend contract. Image code connects the concrete HAL to core.

## Execution and authority

The configured CPU registry holds up to 255 directed xAPIC logical processors. The SMP path resumes Ring 3 contexts in separate or shared process roots; the verification section records tested CPU inventories. All general registers, RIP/RSP/RFLAGS, and FS/GS bases are saved. Each task owns a 16 KiB kernel entry stack with an unmapped lower guard. The HAL updates TSS.RSP0 for entry and restores kernel execution state before returning to the scheduler.

A local xAPIC timer uses calibrated one-shot events to preempt user execution. A growing round-robin task table tracks one Running thread per CPU and retains Dying resources until explicit cleanup succeeds. Invalid saved user state and fatal user exceptions stop the affected task.

The object arena retains strong references. Process-local handles validate generation, kind, and rights; close releases one reference and duplicate only preserves or reduces authority. Experimental int 0x80 calls provide yield, exit, task status/stop, close, and duplicate alongside the IPC operations below.

Endpoint IPC adds Send/Receive/Call/Reply with 32 inline bytes, single-use reply tokens, cancellation, closure, and peer-death cleanup. Notifications coalesce signals. Generational waits preserve completions during Parking and publish saved return values before waking a task. SEND/RECEIVE/WAIT/SIGNAL rights are checked at entry.

EndpointCreate and NotificationCreate require CREATE on an explicitly granted channel factory. Duplication and delegation preserve the same finite account across processes and threads. The creating TaskId owns receiving or waiting. Each channel retains one charge through closure, creator death and stale references until final payload destruction. Serialized settlement removes closed registry roots and closes registry-only channels after accepted grants and completions settle. Device notification roots remain until explicit hardware retirement. [ADR 0014](decisions/0014-native-channel-creation.md) defines these rules; this live-channel count is the first accounting scope, not general resource quotas.

Shared-memory objects own zeroed, non-executable RAM. RO/RW mappings retain a strong whole-object lease, so handle closure does not release mapped frames. Successful unmap retires access before releasing that lease; failed cleanup retains it. The last reference returns backing frames. Discarded arenas retain payloads with forgotten references.

IPC can delegate up to two reduced-rights capabilities. Receivers reserve handle slots before waiting, and acceptance commits grants before any sender completion or death cleanup. Source handles remain valid. Delegation requires TRANSFER; memory map requires READ|MAP and also WRITE for RW.

A checked ACPI RSDP/root/HPET path supplies a retained 64-bit HPET monotonic clock. A bounded ten-millisecond reference interval calibrates the APIC countdown. User entry selects the earlier of a two-millisecond quantum and the next wait deadline. When all timed tasks block, the BSP programs the earliest deadline and uses `sti; hlt; cli`. Timer IRQs record delivery without normal locks; deferred code samples elapsed nanoseconds before completing waits. Early or stale delivery leaves future waits blocked. This is functional idle behavior, not a measurement of power use.

Wait slots retain optional absolute deadlines. Endpoint and notification timeouts detach queues and retire accepted reply tokens before wakeup. All registrations due at the sampled time detach before endpoint progress resumes. Received capability grants commit before sender cleanup, including accepted timeout. ClockRead, SleepUntil, and six timed IPC calls use experimental version 0; timeout returns delivery state and preserves the first completion. Cancellation and task death remove sleep records and deadlines.

Boot tests now load a separately compiled static native ELF from a Limine module. Core validates its page plan; image code builds private RX/RO/RW mappings, zeroed BSS, a guarded user stack, and a private TLS page. Two instances execute ordinary compiled Rust startup/checks and assembly state probes.

Every participating CPU now passes allocation-free x86-64-v3 hardware admission
before BSP/AP initialization and user publication. The check includes the earlier
psABI levels, AVX2/FMA and bounded standard XSAVE/YMM storage. Early admission
does not depend on the bootloader enabling CR4.OSXSAVE; extended entry enables
OSFXSR/OSXSAVE and XCR0 before user execution. There is no platform fallback to
an older ISA. The kernel and existing compiled boot programs retain their
freestanding soft-float target. A separate v3 hard-float target runs the optional
mixed C/C++/Rust ABI probe; the general native runtime remains pending.

Extended tasks eagerly preserve x87/SSE/YMM through bounded standard XSAVE on
admitted hardware. FXSAVE selection remains covered by HAL model tests. Kernel
FP state and raw control state restore before returning. SetTls validates an FS
base; user FSGSBASE access is disabled. Scalar entry still rejects FP and is
checked again after extended execution.

The shared `kernel/image/src/runtime/` now connects these mechanisms in normal boot and guest checks. A generational process registry owns ProcessResources address spaces, shared handles, controls, and retained creation state. A separate thread scheduler owns CPU/FP state, guarded stacks, TLS, and waits. Process resources remain until the final attached thread retires successfully. Fallible ELF/stack creation prepares private resources before runnable publication and preserves cleanup owners on failure.

Normal service registries grow processes, threads, per-process memberships, objects, handles, waits, observations, and VM metadata with available memory. Explicit bounded constructors remain in legacy exhaustion fixtures. Generation histories survive slot reuse and backing reallocation. Creation reserves metadata before runnable publication; wait registration prepares side records before blocking. Ordinary allocation failure returns OutOfResources and retains unpublished or incompletely retired owners. [ADR 0012](decisions/0012-growing-registries.md) records the allocation and growth contract.

Normal boot starts a native supervisor with a versioned immutable startup record. Its explicit root inventory holds image execution, debug delegation and four distinct channel accounts. User-space manifests validate required roles, kinds and maximum rights. Creator-owned LaunchManifest objects contain descriptive source handles; SpawnManifest snapshots and revalidates every source, stages references and installs complete child tables plus READ-only receipts before execution. StartupRead uses indexed words without a raw user mapping; HandleInfo reports a local handle's actual kind/rights. [ADR 0015](decisions/0015-service-startup-and-authority.md) defines this boundary.

The configured Ring 3 registry uses private instance registration and client discovery gateways. Lookup authorization comes from the gateway binding, never a claimed message ID. Registered endpoints have SEND|TRANSFER; clients receive SEND alone. Registration and callback kinds/rights are checked. Bootstrap checkpoint traffic has a separate explicit capability; directory clients cannot request service stop/restart. Replacement withdraws Ready, stops and observes the old process, consumes a fresh generation and registers a fresh endpoint. Malformed queries and expired replies remain per-request failures. The normal supervisor keeps the second service instance alive and monitors it; an unavailable replacement remains unpublished and retries with backoff on the same account.

ResourceBudget accounts now bind production processes before initial execution.
Shared payload pages, open/reserved handles, active/Ready waits and live threads
retain credits through actual retirement. Ordinary children inherit accounts;
ManifestBudget selects another only through ASSIGN|TRANSFER and revalidates its
source at spawn. Threads and replacements do not reset usage. Mapping/table
limits apply per process and include aliases plus the root table. BudgetSelf
reports the current binding without exposing assignment authority. The initial
supervisor now has ten root grants, including three assignable service accounts.
[ADR 0016](decisions/0016-service-resource-budgets.md) separates these bounds from
exact private/kernel RAM and CPU-time accounting.

The prior init/worker programs remain boot-test regressions for legacy two-grant startup, four created threads, exit 42 and retained control/status semantics. A retained ProcessExit result survives VM reaping and does not promise device or deferred cleanup completion. The kernel still provides no root-supervisor restart policy.

Before the initial supervisor, the earlier native bootstrap service receives a shared eight-channel budget, peer image and diagnostics. It checks request/reply, shared exhaustion/reuse, notifications, exit and reconnection with old closed endpoints. All probe resources retire before manifest-based startup. This fixture retains legacy startup independently of the new configured registry.

The original native-smoke ELF remains a boot-test CPU-state fixture. [ADR 0002](decisions/0002-user-address-spaces.md) describes memory ownership, [ADR 0003](decisions/0003-preemption-and-capabilities.md) task execution, [ADR 0004](decisions/0004-waits-ipc-and-idle.md) waits/IPC, [ADR 0005](decisions/0005-shared-memory-and-delegation.md) shared backing, and [ADR 0006](decisions/0006-native-elf-and-extended-state.md) native execution and CPU state.

## Verification

The [native thread matrix](validation/2026-11-03-native-threads/evidence.json)
passes six QEMU configurations, 453 Rust tests and 20 Python tests. Two native
TLS children exercise prepared FS, runtime rounding setup, live-range protection
and join/reclaim; eight managed word children preserve the eleven-thread suite.
Trusted fixtures and core models cover admission, rollback, guard holes, packed
TLS, reuse before kernel reaping and metadata failure. These are model/emulator
results, not complete pthread or standard-library integration.

The [atomic wait matrix](validation/2026-11-03-atomic-waits/evidence.json) passes
six QEMU configurations, 443 Rust tests and 19 Python tests. Four real native
ABI runs cover bounded wake, timeout with HLT, cancellation, mapping reuse and
managed TLS retirement across eleven threads. Trusted boot checks and host
models separately cover cleanup failures, admission and identity lifetime.
The existing private VM, TLS, SIMD, SMP and block-service checks remain required.

The [private VM matrix](validation/2026-11-03-private-vm/evidence.json) passes six
QEMU configurations, 428 Rust tests and 18 Python tests. It covers explicit
memory in four native ABI runs and legacy normal/boot-test regressions with
one/four CPUs. Core failure models cover partial publication, retirement and
metadata OOM; guest checks cover quota admission, zeroing, reuse and final
process cleanup. Normal/all-feature kernel, userspace and scoped host Clippy
and formatting pass. These are functional model/emulator results, not hardware
performance or standard-library integration.

### Previous resource budget validation

The [budget matrix](validation/2026-11-01-budgets/evidence.json) passes all 16 configurations: default-model one-CPU 512-MiB guests, max-model two-/four-CPU 512-MiB guests and max-model two-CPU 129-GiB guests, each in debug/release and boot-tests/normal boot. [Source hashes](validation/2026-11-01-budgets/source-manifest.json) identify 245 unchanged source/configuration/runner inputs. Host tests pass 336 cases: core 261, HAL 58, interfaces 2, libsys 4 and service policy 11. Kernel normal/all-feature Clippy, scoped changed core tests, userspace workspace Clippy and formatting pass. Independent review found no blocker and separately passed 44 targeted model tests.

Both actual Ring 3 clients exhaust/recover handle capacity, retain shared-page usage through a final mapping, create three additional native threads, and prove that closing controls does not refund live-thread credits. Two blocked waits exhaust the account while nonwaiting Signal and Close permit recovery. Both boot modes require all six stage markers; boot checks require six created threads and zero remaining charges at retirement. Trusted guest fixtures prove legacy inheritance, stale/denied assignment, Ready-wait retention, failed-cleanup ownership and shared backing surviving creator reaping. VM models bound aliases and private tables including the root. Prior service, memory, device, SIMD-state and SMP/TLB/growth regressions still pass. The [validation notes](validation/2026-11-01-budgets/README.md) distinguish these resource counts from CPU time, full physical-RAM accounting and real-hardware proof.

### Previous startup and authority validation

The [authority matrix](validation/2026-11-01-authority/evidence.json) passes all 16 configurations: default-model one-CPU 512-MiB guests, max-model two-/four-CPU 512-MiB guests and max-model two-CPU 129-GiB guests, each in debug/release and boot-tests/normal boot. [Source hashes](validation/2026-11-01-authority/source-manifest.json) identify 239 unchanged source/configuration/runner inputs. Host tests pass 319 cases: core 246, HAL 58, interfaces 2, libsys 3 and service policy 10. Kernel normal/all-feature Clippy, scoped core capability Clippy, userspace workspace Clippy and formatting pass.

Actual Ring 3 checks reject forged discovery identity, excess rights, registration/restart through lookup, malformed callbacks and unsupported protocol versions. A deliberately malformed CallCaps registration never becomes Ready; all delivered references close and the complete eight-channel account becomes available again. A separate account remains full while another client completes RPC. Deterministic accepted-call cancellation retires a reply token before the server resumes, and the server continues after InvalidReply. Service replacement preserves old-capability PeerClosed and new-instance RPC. ReceiveFor fixtures prove relative admission, strict absolute expiry and real timer wakeup. Startup failure fixtures preserve private ownership through allocation/publication failure. Normal boot observes the persistent supervisor and replacement; boot-tests retire every object, wait and charge with frame accounting. The [validation notes](validation/2026-11-01-authority/README.md) record intermediate failures and limits; persistent failed-start retry, root-supervisor recovery and hardware performance are not established by this matrix.

All twenty packaged user images preserve their entry and load data after debug-section removal. Existing module/page limits remain. Earlier isolation, device, legacy thread/init, growth and SMP/TLB checks still pass.

### Previous native service validation

The [service matrix](validation/2026-11-01-services/evidence.json) passes all 12 configurations: default-model one-CPU 512-MiB guests and max-model two-CPU 512-MiB/129-GiB guests, each in debug/release and boot-tests/normal boot. [Source hashes](validation/2026-11-01-services/source-manifest.json) identify 225 unchanged source/configuration/runner inputs. Host tests pass 304 cases: core 245, HAL 58, interfaces 1. Normal/all-feature kernel Clippy, scoped core budget/capability Clippy, userspace workspace Clippy and formatting pass.

Actual Ring 3 service peers create and delegate endpoints, exchange request/reply, share one finite account across processes, and exit. Their supervisor checks exhausted-account rollback and reuse, notification coalescing/timeout, denied CREATE/RECEIVE authority, and stale endpoint PeerClosed while a replacement serves requests. Trusted guest checks cover handle/arena/registration failure, pending waits, accepted transfer references, final-charge release and retained device roots. Probe teardown requires zero objects, charges and waits; physical-frame counts allow only measured growth of retained kernel alias tables/metadata. Existing user, device, thread/growth and SMP/TLB checks still pass. The [validation notes](validation/2026-11-01-services/README.md) separate these results from general quotas, stable service identities and hardware proof.

### Previous RAM validation

The fixed four-GiB bitmap limit is removed. Boot metadata is sized from the normalized Usable-frame count, carved from a contiguous Usable range, excluded through re-normalization, zeroed before typed references are created, and retained for the pool lifetime. A bootstrap guard prevents later initialization attempts from zeroing active metadata. Core bounded constructors remain available for failure tests. [ADR 0013](decisions/0013-runtime-frame-metadata.md) records the contract.

The [final RAM matrix](validation/2026-11-01-ram/evidence.json) passes all 20 configurations: max-model two-CPU guests at 8/128/129 GiB and 512 MiB, plus default-model one-CPU 512-MiB guests, each in debug/release and boot-tests/normal boot. [Source hashes](validation/2026-11-01-ram/source-manifest.json) identify the tested source and runner inputs; documentation and generated artifacts are excluded. Host tests pass 298 cases: core 239, HAL 58, interfaces 1. Kernel normal/all-feature Clippy, scoped host pool Clippy, and formatting pass.

Every 129-GiB guest manages more than 128 GiB after reservations (approximately 128.96 GiB). Boot tests read/write/zero/reuse real owned pages at or above physical 4 GiB and 128 GiB when present, and use high physical backing for remote TLB retirement and missing-ACK retention. The metadata reservation is excluded from allocation, with exact before/after frame accounting. All earlier user/IPC/thread/device checks and 127-thread growth complete. The maximum sampled emulator-process resident memory was below 144 MiB across the matrix. Demand-backed guest size, actually managed frames, sampled host RSS, and sparse-page access are distinct observations; this is not a full-memory sweep or physical 128-GiB DRAM validation.

The [first 129-GiB attempt](validation/2026-11-01-ram/prior-low-reservation.debug.log) reached high-frame checks but stopped at the existing alias-preparation rollback assertion. That fixture had selected low reservations through the cursor-based general allocator. It now explicitly allocates within [0, 2 MiB), preserving the intended backing premise and the original TableCapacity/cleanup assertions. No scheduling deadline or quantum was increased.

### Previous scaling validation

The [final scaling matrix](validation/2026-11-01-scaling/evidence.json) passes all 20 standard MTTCG configurations: 65/8/2 processors with the max CPU model, and one processor with default/max models, each in debug/release and boot-tests/normal boot. [Source hashes](validation/2026-11-01-scaling/source-manifest.json) identify the tested code. Compiled inputs and the runner stayed unchanged through the matrix; documentation-only updates are listed separately in the evidence.

The scaling baseline passed 291 host tests: core 232, HAL 58, and interfaces 1. Kernel normal/all-feature Clippy, userspace production workspace Clippy, and formatting pass. Scoped host core and all HAL target Clippy checks pass; the pre-existing range.rs:43 identity_op remains outside the clean host-test lint scope. The 65-CPU full guest suite checks shared-root progress and migration, CPU64 TLB retirement, missing-ACK retention, fault/call policy, native growth, and final resource accounting.
The new native scale fixture requires seven processes, 127 threads, 127 real timed waits, and more than 512 kernel stack-region pages. It includes private TLS/shared-state checks, membership-publication allocation failure and retry, and explicit teardown. Host growth tests cover larger task, process, member, object, handle, wait, observer, reply, and VM tables; allocator-null tests check ordinary failure and ownership preservation.

The shipped runner uses standard MTTCG and ordinary `boot-tests`; multi-CPU runs retain the shared-root simultaneous-progress and migration fixture. The fault/call fixture lets both jobs finish their unchanged quantum through a boot-only full-batch helper. UD2 executes independently of a peer barrier, while the sibling executes a real int 0x80. Captured RIP, terminal states, and suppressed debug output still establish process-wide policy. Normal production scheduling keeps its early peer-reschedule IPI; production HAL timer behavior, quanta, and deadlines are unchanged.

Preliminary 128-CPU MTTCG exited with signal 9 before kernel output; a separate 255-CPU MTTCG run reached its 90-second host deadline without kernel output. Their causes are not established. In a controlled 30-second, 255-CPU probe, stopped BSP snapshots showed pre-long-mode execution and CR3=0 while debugcon remained empty. That locates those snapshots before Kryo entry; it does not explain the separate exit or timeout. Temporary serialized and instruction-count diagnostic builds also failed and are not shipped test modes. The first serialized attempt failed to capture both fault/call causes in one batch. A 65-CPU diagnostic with a small translation cache failed a scalar progress check; the later default-cache success does not establish the cache setting as its cause. Failure logs and the controlled probe are retained in the scaling evidence directory. Guest operation above 65 CPUs remains unconfirmed.

### Historical baseline

The following table records the completed [device and CPU expansion baseline](validation/2026-11-01-devices/evidence.json), before growing registries. Its 24 configurations cover 1/2/8/64 CPUs, debug/release, and boot-tests/normal boot; default and max CPU models run at 1/2 CPUs, max at 8/64. [Source hashes](validation/2026-11-01-devices/source-manifest.json) identify that earlier source. These results remain regression evidence, not verification of later source changes. The earlier [four-CPU SMP matrix](validation/2026-11-01-smp/evidence.json) is also historical.

| Check | Result |
| --- | --- |
| no_std workspace check and Rust formatting | Passed |
| Workspace Clippy, normal and all-feature kernels, warnings denied | Passed |
| Native userspace workspace builds | Passed in debug/release across the matrix with isolated build flags; user production Clippy and formatting pass |
| Host tests | 272 passed: core 215, HAL 56, interfaces 1 |
| Host Clippy scope | Changed core task/device targets and all HAL targets pass; whole-workspace all-target check reports a pre-existing identity_op in range.rs:43 |
| Core task/object/capability tests | Ownership, cleanup retry, exhaustion, stale generations, slot retirement, reference limits, and rights pass |
| Existing VM failure models | Allocation/publication failures, alias quarantine, inactive-state checks, and retirement ordering passed |
| QEMU TCG debug/release | All boot markers passed in each profile |
| User memory and entry regressions | 16 prior user cases and 128 mapping cycles passed |
| Noncooperative preemption | Two infinite-loop tasks each received four slices without a yield; caller/callee register and memory counters progressed |
| Capability calls | Three forbidden operations and one closed-handle lookup rejected; reduced duplication, close, yield, target stop, and exit completed |
| Invalid saved context | A task set RSP=0 and looped; resume rejected it while its peer continued |
| Pending timer shutdown | A one-shot timer queued vector 0xe0 with IRQs off; shutdown drained it before MMIO removal |
| Task reclamation | User mappings, entry stacks, and objects/handles reclaimed; task-stage frame count restored; platform tables remain image-owned |
| Wait lifetime | 10 new host tests cover early/late completion, first-result wins, ownership, exhaustion, reuse, abandonment, and generation retirement |
| Endpoint/notification models | 17 new host tests cover rendezvous, FIFO, bounds, reply identity, cancellation, owner/caller death, closure, and coalescing |
| Guest IPC | Eight scenarios passed, including 32-byte request/reply, Send, denied Receive, reply replay, queued/accepted cancellation, closure, and owner death |
| Guest idle | Two notification cases blocked all remaining tasks, halted, then woke on an IRQ and resumed; a pending-before-HLT case also passed |
| Shared ownership models | 12 new VM tests cover cross-space leases, creation/map failure, alias policy, executable rejection, and cleanup retry; two arena tests cover forgotten backing |
| Transfer models | 18 reservation/delegation tests, nine IPC metadata tests, and five combined endpoint/handle/wait tests passed |
| Guest shared memory | Two capabilities shared three pages; zeroing, a page-tail marker, second-page data, mapped lifetime after all handles closed, and reduced rights checked |
| Acceptance ordering | Received grants committed when the sender's Sent slot sorted first; cleanup could not discard accepted references |
| Capacity rollback | A two-capability request to a zero-capacity receiver failed before acceptance; source close and an ordinary-message retry succeeded |
| Shared RO fault | A separate user write to RO shared RAM produced #PF vector 14/error 7 at 0xc00000; task cleanup reclaimed the last mapping |
| ELF parser | 23 new host tests cover malformed/truncated files, checked ranges, overlap, flags, entry rules, padding/BSS, and generated mutations |
| Extended state | Five new HAL tests cover initialization/alignment, CPU/layout selection, bounds, XCR0 policy, and FS setter behavior |
| Native module | Both profiles produce ET_EXEC with three load segments/four pages; the actual core parser accepted both artifacts |
| Native execution | Two module-backed programs passed data/RO/BSS checks, private result checks, real preemption, native yields, and Exit |
| CPU configurations | Debug/release passed on default FXSAVE and max-CPU XSAVE/AVX configurations |
| FP/TLS isolation | Distinct x87/control/MXCSR/XMM and optional YMM state plus FS data survived switching; invalid TLS updates preserved the valid base |
| Restoration and cleanup | Scalar FP traps still worked after extended execution; all per-native-task allocations returned before final ABC |
| Clock and firmware models | 12 new HAL tests cover checked ACPI discovery, HPET capabilities/conversion/restoration, and APIC calibration/countdown rules |
| Deadline and timeout models | 13 new core tests cover absolute bounds, ordering, retirement, first completion, reply tokens, notification races, and batching expiry before acceptance |
| Guest time | 11 scenarios passed on all four CPU/profile runs: sleep, queued/accepted timeout, notification event/expiry, cancellation and slot reuse, expired admission, receive retry, capability timeout, and early IRQ |
| Capability timeout | Queued transfer retains source authority; accepted transfer remains mapped after caller timeout and source closure, while a late reply fails |
| Early IRQ | A queued old APIC vector interrupts idle, but a future sleep stays pending and Blocked until its actual deadline |
| Process lifecycle models | 10 new tests cover retained first result, early/late observers, Parking, cancellation/timeout, observer death, cleanup retry, and task-slot reuse |
| Owned capability publication | Two new tests cover moving a trusted new control into reserved capacity, exact reference retention, and failed reservation retry |
| Launch rejection and rollback | Nine guest cases reject malformed/overlapping images and stack conflicts; unpublished VM owners remain until explicit cleanup and frame counts restore |
| Startup failure transactions | Five guest cases reject image rights/format and handle/object/task exhaustion without publishing a child or leaking private resources |
| User supervision | Actual init/worker ELFs execute twice, retain an old control across slot reuse, observe exit 42, and finish full test cleanup |
| Normal boot | Debug/release on default/max CPUs run init without boot-tests, with two distinct child generations and user-originated readiness |
| Process/thread ownership | 10 new registry tests plus generation-wrap coverage retain process resources through member cleanup; three suspended-task tests verify publication before dispatch |
| Guest thread lifecycle | Nine cases cover invalid entries, self-join, per-process capacity, guard-fault isolation, group exit, last-thread exit, timed join, object exhaustion, and failed retirement retry |
| Shared process state | Two compiled workers each create two successive threads; shared atomics/handles, private TLS/x87/SSE/optional AVX, old controls, and joins pass |
| Real thread preemption | All four created threads and both worker main threads record timer preemption before test teardown |
| Retirement retry | Injected backing restoration failure leaves a Dying member and unpublished mapping retained; creation is denied until retry retires all five private user pages |
| SMP ownership models | Seven core tests cover CPU owners, wrong/stale returns, duplicate prevention, round-robin selection, and retirement after all CPU handoffs |
| CPU-local HAL | Private GDT/TSS/IST, entry state, kernel FP areas, APIC timers, CPU identities, and IRQ epochs |
| SMP user execution | Fresh shared-memory barriers and counters progress on distinct CPUID identities; thread migration preserves private FS/TLS |
| Cross-CPU waiting and failure | A sleeping thread blocks while its sibling progresses and later resumes; fatal process policy suppresses a sibling call captured in the same batch |
| Remote TLB retirement | Every online CPU primes a supervisor alias, acknowledges removal/replacement, and observes new backing before old frames return |
| Missing acknowledgement | Injected remote ACK loss retains the removed backing and blocks reuse/dispatch; retry completes retirement; failed stack publication cleans up retained aliases |
| AP teardown | Workers retire local timers and publish offline before shared MMIO and region tables are removed |
| CPU capacity | 64 logical slots; constant-time APIC reverse lookup, sparse IDs, slot63 and APIC254; broadcast255 rejected |
| Early AP idle | A retained supervisor APIC branch lets prepared APs halt until final startup; static table pages remain image-owned |
| Device policy models | Eight tests cover register windows, owner generations, rights, overflow/alignment, and failed cleanup |
| Real user driver probe | A separately compiled ELF uses permitted HPET words and actual untimed/timed notification waits; forbidden words, values, rights, and stale handles are rejected |
| Device death | Armed process stop and IRQ-consumer ThreadExit revoke hardware authority; a surviving sibling observes PeerClosed |
| Device retirement | Actual queued D0 drains before reuse; injected failure preserves process resources and exclusive claim until retry |
| Device expiry | A real queued IRQ arrives after a synthetic core wait deadline; TimedOut wins and the pending signal completes the next wait |
| Registration capacity | Full channel registration returns its owned notification reference without enabling or discarding hardware authority |
| Kernel continuation | Existing user-stage accounting, framebuffer path, and final ABC completed |

CPU state is private per registered CPU; the IDT is immutable and loaded on each CPU. The BSP retains process/object/IPC/VM policy. Owned mailbox jobs run user contexts concurrently, then all CPUs restore the boot root before native calls or mapping changes. Kernel mapping updates use global TLB epochs and acknowledgements. [ADR 0010](decisions/0010-smp-execution.md) records the execution, wakeup, and retirement rules.

The first SMP boot faulted in Limine's HHDM trampoline after the BSP made HHDM RAM NX. APs now acknowledge a kernel-text parking loop before that policy change. [Initial log](validation/2026-11-01-smp/prior-ap-nx.log) and [exception trace](validation/2026-11-01-smp/prior-ap-nx-trace.log) identify the instruction-fetch fault. The first parallel fixture then caught absent CPU migration: queued reschedule IPIs could end new slices before progress. A notification-completion handshake, start barrier, pending-IPI drain, and fresh entry-time quantum corrected it. [Prior migration log](validation/2026-11-01-smp/prior-migration.log) is retained.

The prior device baseline had two processes, two threads per process, four scheduled threads, and 64 online CPU slots. Its [storage measurements](validation/2026-11-01-devices/cpu-storage.json) describe that earlier configuration. Current service registries grow independently of the 255-CPU hardware registry. CPU-local static storage and actual-CPU Limine stacks still consume memory even when fewer user threads are runnable.

The [mediated device contract](decisions/0011-device-authority.md) keeps HPET registers mapped only in the kernel and uses process-bound READ/WRITE authority plus a thread-bound IRQ notification. There is no arbitrary physical map, user MMIO page, user PCI configuration or DMA API. The normal bootstrap probe is a validation service, not a general device manager.

The first 64-CPU full regression exposed timing assumptions while prepared APs spun throughout BSP setup. [Initial log](validation/2026-11-01-devices/prior-spin.debug.log) and [controlled diagnostic](validation/2026-11-01-devices/prior-scalar.debug.log) are retained. The scalar fixture now permits legal timer preemption before requiring an actual #NM. Legacy IPC/time fixtures use real producer completion, blocked-state observations and notification handshakes to establish ordering; relative sleeps and yield counts no longer stand in for those states. Deadlines, expected results and execution budgets remain unchanged. Early platform setup gives APs an owned UC/NX APIC mapping and interruptible HLT before memory policy and BSP checks. Three aligned image-owned table pages retain the branch; they never enter or return to the frame pool. Kernel alias initialization explicitly validates this one MMIO-only branch. Final startup still reloads protection state and flushes translations before online publication.

The historical memory/user sequence retained one explicitly owned HHDM split-table frame. Current split-table ownership grows with demand; final frame checks include the exact retained table count and the heap pages backing its owner metadata. They require exact restoration of the other allocations. Growing registry generations persist until their owning registry drops; additional shared stack tables persist until final branch retirement. Table permissions and guard presence are checked by the guest fixtures.

The first four-thread-registry debug run exceeded the prior 256 KiB bootstrap stack. Its selected function prologues alone totalled 266,536 bytes, matching a stack pointer below the boot page-table page and #PF/#DF/triple fault. [Failure log](validation/2026-11-01-threads/prior-debug.log), [trace](validation/2026-11-01-threads/prior-triple-fault.log), and [stack notes](validation/2026-11-01-threads/stack-analysis.json) are retained. Debug BSP stack capacity is now 512 KiB; release remains 256 KiB, and each guarded thread entry stack remains 16 KiB. All eight final configurations passed. Prologue subtotals are not exhaustive bounds or physical high-water measurements.

An initial debug spawn-failure fixture exceeded the 256 KiB bootstrap stack. The [guest log](validation/2026-11-01-startup/prior-debug.log) and [exception trace](validation/2026-11-01-startup/prior-triple-fault.log) are retained: RSP reached the boot page-table page before instruction-fetch #PF and triple fault. Separating fixture preparation, execution, and teardown reduced the caller frame from about 86 KiB to 20,544 bytes. That stage retained its 256 KiB request. All eight final boots passed. [Stack notes](validation/2026-11-01-startup/stack-analysis.json) distinguish disassembly frame subtotals from a complete bound or hardware high-water measurement.

An initial task run timed out after the capability checks while a count-1 periodic timer was used to force pending delivery. The final regression uses a bounded one-shot timer and passes both profiles. The [earlier log](validation/2026-10-31-tasks/prior-timeout.log) is retained; the exact cause of that timeout was not established.

An initial HPET discovery run halted before clock initialization while the parser required a GAS width of 64. The final parser also accepts unspecified width 0, used by QEMU firmware. The [earlier log](validation/2026-11-01-time/prior-discovery.log) is retained; the final four runs passed after this adjustment.

Historical records remain available for [native init](validation/2026-11-01-startup/evidence.json), [monotonic deadlines](validation/2026-11-01-time/evidence.json), [native execution](validation/2026-10-31-native/evidence.json), [shared memory](validation/2026-10-31-shared/evidence.json), [inline IPC](validation/2026-10-31-ipc/evidence.json), [task execution](validation/2026-10-31-tasks/evidence.json), [user entry](validation/2026-10-31-userspace/evidence.json), [memory bootstrap](validation/2026-10-31-memory/evidence.json), and the [layout migration](validation/2026-10-30-layout-notes.md). Host tests, QEMU execution, and hardware measurements are separate evidence. The runner stops QEMU from the host after the final marker; it does not verify guest shutdown.

## PCI discovery and DMA storage

The bootstrap retains a read-only PCI segment-zero inventory through conventional
CF8/CFC access. Local IRQ exclusion and a HAL lock serialize selector/data pairs.
All 256 bus numbers are scanned, including additional multifunction functions.
Snapshots contain identity, class, command and raw BARs; they grant no mapping or
configuration authority. Device command bits are unchanged, including any
bus-master enable left by firmware. BAR sizing, ECAM, other segments, hotplug and
general driver assignment remain pending.

Core DMA buffers own constrained contiguous runs and distinct device-address
descriptions. They initialize backing before publication and explicitly quiesce,
unmap and release. Failure returns a retained owner for retry; abandonment does
not recycle backing or tear down a live translation. Constructor rollback follows
the same rule. The private boot check zeroes, dirties and retires real RAM below
four GiB, with exact frame restoration except measured retained alias tables.
The unpublished-buffer check remains separate from the private hardware transport
below. There is no general user DMA API, pin accounting or IOMMU domain.

[ADR 0017](decisions/0017-pci-discovery-and-dma-storage.md) and
[storage validation](validation/2026-11-02-storage/README.md) record the foundation.

## Ring 3 block commands

An explicitly enabled native driver now executes actual VirtIO block read,
write, flush and readback. HAL uses modern PCI configuration-window access;
core snapshots offset-only buffer plans into private split queues and bounce
RAM. Process-bound Device authority and READ/WRITE memory rights are checked
before access. Foreign processes with copied device capabilities are denied.
Malformed ranges, unsupported flags, busy submits and stale tickets are rejected.
User descriptor changes after submission do not change the accepted plan.

Reset completion precedes DMA return and pending-memory release. A process may
exit with an accepted read; cleanup retains every owner until reset succeeds.
Boot tests inject one declined cleanup attempt and verify retained resources.
The Ring 3 driver also resets, renegotiates, restarts and rejects an old ticket.
Default images do not enable the block probe; explicit boot opt-in and a signed
disposable disk precede any test write. Read-only mode requires an actual device
write error and unchanged backing bytes.

This is a validation driver using native transport calls, not a persistent block
IPC service or a filesystem mount. Completion polling, one request, three
descriptors and private 8-KiB payload bounds are the current bootstrap scope.
No IOMMU or isolation against malicious hardware is claimed. See
[ADR 0018](decisions/0018-mediated-virtio-block-transport.md) and
[block validation](validation/2026-11-02-block/README.md).

BetAFS remains the filesystem target and keeps its Ring 3 std dependency.
The [adapter contract](../userspace/filesystems/betafs/README.md) preserves 4096-byte
blocks, read-your-writes and durable flush. Cryolite Rust target/std support,
libc, C++ runtime/libraries and their shared platform services are pending;
[ADR 0019](decisions/0019-native-runtime-and-standard-libraries.md) records the
order. A no_std BetAFS rewrite is not the integration direction.

## Initial block IPC service

The configured block service now runs in the normal supervisor's shared runtime
graph and remains waiting after its validation clients exit. A separate read,
write and test-control endpoint conveys each authority. Version-one messages
carry fixed 4096-byte memory loans; clients supply no VirtIO commands. All grants
retire before reply, including malformed and fire-and-forget messages.

Suspended process preparation installs Device ownership and immutable startup
records before Runnable publication. Driver/client accounts and channel payloads
remain bounded. An accepted cancelled read retains its loan until cleanup; a
subsequent Info reply establishes the single-threaded service's sequencing barrier.
Late reply rejection is nonfatal. Boot tests use a control client for graceful
stop and verify complete resource retirement. Normal idle has an explicit device
cleanup retry deadline independent of supervisor activity.

The graph remains opt-in on the signed disposable disk. General publication,
device reassignment/restart, storage IRQs and wait-any remain pending; the server
rotates three gates using one-millisecond waits. See
[ADR 0020](decisions/0020-initial-block-service.md) and
[service validation](validation/2026-11-02-block-service/README.md).

Native images already use ELF64 AMD64 static ET_EXEC. Current bootstrap Rust
services still use the soft-float bare-metal target and register-based startup.
Separate v3 hard-float probes validate SysV startup; the static mlibc profile
adds the first provider crt/TCB. General hosted ABI and standard-library ports
remain incomplete. [ADR 0021](decisions/0021-native-elf-abi.md) distinguishes format and ABI.

## Limits

- One to 255 xAPIC logical processors with directed APIC IDs 0..254 and multiword CPU sets, four-level paging, PCID off, and serialized BSP page-table policy. Larger CPU inventories are rejected before HHDM protection changes. No hotplug, x2APIC, affinity, topology/NUMA policy, per-CPU run queues, or parallel kernel syscall processing. IRQs remain disabled during policy work; idle, drain, and user execution use controlled IRQ windows.
- xAPIC only. Time requires an ACPI-described 64-bit HPET, bounded firmware tables, and BSP configuration ownership with shared registered-CPU reads. PIT/PM/TSC fallbacks, 32-bit counter extension, suspend/resume, and hardware clock/latency validation are pending. Calibration does not guarantee scheduling latency while kernel IRQs remain off.
- Scalar entry traps FP; extended entry supports x87/SSE and optional AVX in a private 1,024-byte standard state area. XCR0 components beyond bits 0..2, AVX-512, AMX, and supervisor state are unsupported. Kernel code remains soft-float.
- Legacy native tests use scalar Rust compilation and manual SIMD assembly. The optional v3 hard-float C/C++/Rust probe uses SysV startup, static ELF TLS and runtime-owned native thread creation. Complete libc/std, dynamic linking, dynamic TLS and pthread integration remain pending.
- Initial executable policy is static little-endian AMD64 ET_EXEC only, at most 64 program headers/eight load segments and a 1-MiB boot-module cap. Legacy fixtures admit 32 load pages; SysV native boot images admit up to 4096, further bounded by their mapping account. Per-process mapping/table budgets still apply. This is not ICE execution or general ELF compatibility.
- Normal metadata grows through fallible allocation within configured service accounts. Shared pages, handles, waits, live threads, explicit private VM pages and reserved virtual pages have persistent charges; VM mapping/table limits are per process. Exact kernel-metadata byte accounting, private-RAM accounting for ELF/stacks/legacy TLS and CPU-time budgets remain pending. These are account policy limits, not a fixed kernel-wide process/thread count. Endpoint RECEIVE and notification WAIT remain thread-bound. General process hierarchy, multi-receiver IPC, dynamic service inventory, supervisor recovery and WAIT delegation remain pending. The configured registry is scoped to one supervisor lifetime.
- Frame metadata scales with the Usable RAM plan and has no fixed four-GiB or 128-GiB bitmap cap. Boot discovery still accepts at most 256 raw map entries and 288 normalized ranges, and metadata requires one contiguous Usable span. CPU physical-address width and canonical HHDM bounds remain enforced. The shared entry-stack branch has a one-GiB virtual-address budget, roughly 52,000 guarded cells; other resources can exhaust sooner. Private user VM metadata grows instead of retaining the earlier 16-mapping/64-table fixture limits.
- Private mappings have unique backing; shared mappings retain the whole memory object and its shared-page charge. Native memory objects allow up to eight pages each, subject to the account total. Buffer loans, bulk queues, pin accounting and revocation remain pending.
- HHDM split tables and their growable ownership metadata remain kernel-owned, without coalescing. Bootloader-reclaimable/ACPI memory and active boot tables remain reserved.
- Small heap objects share Talc arenas over physically contiguous backing runs. Fragmentation can still cause large backing requests to fail despite free RAM. The heap uses one IRQ-excluded mutex; CPU-local caches, virtually contiguous backing, stable calls, multi-object waits and DMA isolation remain pending. IPC carries inline data and up to two copied capabilities; linear move and receiver-owner migration are unsupported. Wait/object changes retain serialized BSP ownership of one persistent wait table between execution batches.
- Task, membership, object, handle, and deadline selection use linear scans. Capacity growth does not provide scalable scheduling or parallel kernel policy. Deadline selection scans the growing wait table. Timer scheduling is connected by the shared normal/test runtime. Device IRQ routing, NMI work scheduling, suspend/resume, general C-state policy, and physical power measurements remain pending. HPET counter wrap, reset, or nanosecond exhaustion returns an error.
- ICE execution, BetAFS I/O, terminal/Velocity, and KDE integration remain unimplemented.

## Next work

Extend the restricted std profile with independent random suppliers, fuller native filesystem APIs,
standard-stream startup and broader Unix contracts. Basic std file operations now use service-backed C descriptors. Descriptor metadata and positioned I/O now have native service contracts;
directory operations and actual durable synchronization still need their backends.
Provider C11 thread/TCB lifecycle,
static TLS and the fallible Talc heap are implemented; pthreads, dynamic TLS
and complete standard libraries remain separate work. Connect the initial
block service to VFS, the native runtime and BetAFS. Add storage IRQ
completion and general device assignment while retaining the private queue
boundary. The current HPET grant remains a bootstrap device path. CPU-time
accounting must measure individual CPU jobs; preemption counts and BSP batch wall
time do not establish service fairness. SIMD remains user-state preservation for
x87/SSE/YMM on admitted v3 hardware with kernel soft-float. Efficient heap backing, scalable event
waits, dynamic configuration, complete private/kernel memory accounting and supervisor
recovery remain follow-ups. x2APIC, memory hotplug and NUMA remain separate.
General user runtimes, PTY and compatibility execution are required before
ordinary applications. The [roadmap](roadmap.md) lists terminal, Velocity and KDE
checkpoints.
