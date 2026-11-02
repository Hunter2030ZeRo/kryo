# Implementation status

Snapshot: 2026-11-01. The [design contract](design-contract.md) defines requirements; this file records implemented behavior.

## Memory and boot

- Limine memory/HHDM/image/framebuffer requests, four-level paging, and a 512 KiB debug / 256 KiB release Limine entry stacks.
- GDT/TSS/IST and IDT setup, emergency diagnostics, breakpoint return, debug output, and a 32bpp framebuffer writer.
- Checked physical ranges, reservation subtraction, and a one-time bitmap frame pool with allocation tokens. Bitmap and summary storage is sized from Usable RAM and reserved before pool/heap activation.
- HHDM access, zeroing, and BSP scratch mappings.
- User spaces with growing mapping/table metadata owning their root, intermediate tables, and data frames.
- RO/RW/RX mappings, initialization before publication, inactive-space mutation, and explicit cleanup.
- Supervisor-only kernel mappings, HHDM NX policy, and read-only aliases for executable/read-only user frames.
- Huge HHDM splitting with retained kernel table ownership.
- A supervisor-only kernel region for uncached APIC access and guarded per-task entry stacks; added leaf tables extend it up to a one-GiB address budget.
- A fallible metadata heap retaining contiguous frame-run tokens through deallocation. Whole-page allocation and physical fragmentation remain limitations.

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

Extended tasks eagerly preserve x87/SSE through FXSAVE64 or bounded standard XSAVE, with optional AVX. Kernel FP state and raw control state restore before returning. SetTls validates an FS base; user FSGSBASE access is disabled. Scalar entry still rejects FP and is checked again after extended execution.

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

### Current resource budget validation

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

The [mediated device contract](decisions/0011-device-authority.md) keeps HPET registers mapped only in the kernel and uses process-bound READ/WRITE authority plus a thread-bound IRQ notification. There is no arbitrary physical map, user MMIO page, PCI configuration, or DMA API. The normal bootstrap probe is a validation service, not a general device manager.

The first 64-CPU full regression exposed timing assumptions while prepared APs spun throughout BSP setup. [Initial log](validation/2026-11-01-devices/prior-spin.debug.log) and [controlled diagnostic](validation/2026-11-01-devices/prior-scalar.debug.log) are retained. The scalar fixture now permits legal timer preemption before requiring an actual #NM. Legacy IPC/time fixtures use real producer completion, blocked-state observations and notification handshakes to establish ordering; relative sleeps and yield counts no longer stand in for those states. Deadlines, expected results and execution budgets remain unchanged. Early platform setup gives APs an owned UC/NX APIC mapping and interruptible HLT before memory policy and BSP checks. Three aligned image-owned table pages retain the branch; they never enter or return to the frame pool. Kernel alias initialization explicitly validates this one MMIO-only branch. Final startup still reloads protection state and flushes translations before online publication.

The historical memory/user sequence retained one explicitly owned HHDM split-table frame. Current split-table ownership grows with demand; final frame checks include the exact retained table count and the heap pages backing its owner metadata. They require exact restoration of the other allocations. Growing registry generations persist until their owning registry drops; additional shared stack tables persist until final branch retirement. Table permissions and guard presence are checked by the guest fixtures.

The first four-thread-registry debug run exceeded the prior 256 KiB bootstrap stack. Its selected function prologues alone totalled 266,536 bytes, matching a stack pointer below the boot page-table page and #PF/#DF/triple fault. [Failure log](validation/2026-11-01-threads/prior-debug.log), [trace](validation/2026-11-01-threads/prior-triple-fault.log), and [stack notes](validation/2026-11-01-threads/stack-analysis.json) are retained. Debug BSP stack capacity is now 512 KiB; release remains 256 KiB, and each guarded thread entry stack remains 16 KiB. All eight final configurations passed. Prologue subtotals are not exhaustive bounds or physical high-water measurements.

An initial debug spawn-failure fixture exceeded the 256 KiB bootstrap stack. The [guest log](validation/2026-11-01-startup/prior-debug.log) and [exception trace](validation/2026-11-01-startup/prior-triple-fault.log) are retained: RSP reached the boot page-table page before instruction-fetch #PF and triple fault. Separating fixture preparation, execution, and teardown reduced the caller frame from about 86 KiB to 20,544 bytes. That stage retained its 256 KiB request. All eight final boots passed. [Stack notes](validation/2026-11-01-startup/stack-analysis.json) distinguish disassembly frame subtotals from a complete bound or hardware high-water measurement.

An initial task run timed out after the capability checks while a count-1 periodic timer was used to force pending delivery. The final regression uses a bounded one-shot timer and passes both profiles. The [earlier log](validation/2026-10-31-tasks/prior-timeout.log) is retained; the exact cause of that timeout was not established.

An initial HPET discovery run halted before clock initialization while the parser required a GAS width of 64. The final parser also accepts unspecified width 0, used by QEMU firmware. The [earlier log](validation/2026-11-01-time/prior-discovery.log) is retained; the final four runs passed after this adjustment.

Historical records remain available for [native init](validation/2026-11-01-startup/evidence.json), [monotonic deadlines](validation/2026-11-01-time/evidence.json), [native execution](validation/2026-10-31-native/evidence.json), [shared memory](validation/2026-10-31-shared/evidence.json), [inline IPC](validation/2026-10-31-ipc/evidence.json), [task execution](validation/2026-10-31-tasks/evidence.json), [user entry](validation/2026-10-31-userspace/evidence.json), [memory bootstrap](validation/2026-10-31-memory/evidence.json), and the [layout migration](validation/2026-10-30-layout-notes.md). Host tests, QEMU execution, and hardware measurements are separate evidence. The runner stops QEMU from the host after the final marker; it does not verify guest shutdown.

## Limits

- One to 255 xAPIC logical processors with directed APIC IDs 0..254 and multiword CPU sets, four-level paging, PCID off, and serialized BSP page-table policy. Larger CPU inventories are rejected before HHDM protection changes. No hotplug, x2APIC, affinity, topology/NUMA policy, per-CPU run queues, or parallel kernel syscall processing. IRQs remain disabled during policy work; idle, drain, and user execution use controlled IRQ windows.
- xAPIC only. Time requires an ACPI-described 64-bit HPET, bounded firmware tables, and BSP configuration ownership with shared registered-CPU reads. PIT/PM/TSC fallbacks, 32-bit counter extension, suspend/resume, and hardware clock/latency validation are pending. Calibration does not guarantee scheduling latency while kernel IRQs remain off.
- Scalar entry traps FP; extended entry supports x87/SSE and optional AVX in a private 1,024-byte standard state area. XCR0 components beyond bits 0..2, AVX-512, AMX, and supervisor state are unsupported. Kernel code remains soft-float.
- The native test uses scalar Rust compilation and manual SIMD assembly. A general user target, hard-float ABI, libc/std, dynamic linking, ELF TLS templates, and argc/argv/envp/auxv startup are pending. FS setup is explicit.
- Initial executable policy is static little-endian AMD64 ET_EXEC only, at most 64 program headers/eight load segments/16 pages and a 1-MiB boot-module cap. The page budget was expanded after the budget-aware supervisor measured nine load pages. This is not ICE execution or general ELF compatibility.
- Normal metadata grows through fallible allocation within configured service accounts. Shared pages, handles, waits and live threads have persistent charges; VM mapping/table limits are per process. Exact kernel-metadata byte accounting, aggregate private-RAM and CPU-time budgets remain pending. These are account policy limits, not a fixed kernel-wide process/thread count. Endpoint RECEIVE and notification WAIT remain thread-bound. General process hierarchy, multi-receiver IPC, dynamic service inventory, supervisor recovery and WAIT delegation remain pending. The configured registry is scoped to one supervisor lifetime.
- Frame metadata scales with the Usable RAM plan and has no fixed four-GiB or 128-GiB bitmap cap. Boot discovery still accepts at most 256 raw map entries and 288 normalized ranges, and metadata requires one contiguous Usable span. CPU physical-address width and canonical HHDM bounds remain enforced. The shared entry-stack branch has a one-GiB virtual-address budget, roughly 52,000 guarded cells; other resources can exhaust sooner. Private user VM metadata grows instead of retaining the earlier 16-mapping/64-table fixture limits.
- Private mappings have unique backing; shared mappings retain the whole memory object and its shared-page charge. Native memory objects allow up to eight pages each, subject to the account total. Buffer loans, bulk queues, pin accounting and revocation remain pending.
- HHDM split tables and their growable ownership metadata remain kernel-owned, without coalescing. Bootloader-reclaimable/ACPI memory and active boot tables remain reserved.
- The initial heap uses whole-page, physically contiguous runs. Small allocations waste space and fragmentation can cause failure despite free RAM. Slab allocation and virtually contiguous heap backing remain pending, along with stable calls, multi-object waits, and DMA isolation. IPC carries inline data and up to two copied capabilities; linear move, reply-capability transfer, and receiver-owner migration are unsupported. Wait/object changes retain serialized BSP ownership of one persistent wait table between execution batches.
- Task, membership, object, handle, and deadline selection use linear scans. Capacity growth does not provide scalable scheduling or parallel kernel policy. Deadline selection scans the growing wait table. Timer scheduling is connected by the shared normal/test runtime. Device IRQ routing, NMI work scheduling, suspend/resume, general C-state policy, and physical power measurements remain pending. HPET counter wrap, reset, or nanosecond exhaustion returns an error.
- ICE execution, BetAFS I/O, terminal/Velocity, and KDE integration remain unimplemented.

## Next work

Connect native device discovery, IRQ authority and bounded DMA to the first storage path, then VFS/BetAFS. The current HPET grant is a bootstrap device path, not a general device manager. CPU-time accounting must measure individual CPU jobs; preemption counts and BSP batch wall time do not establish service fairness. SIMD remains user-state preservation for x87/SSE/optional AVX with kernel soft-float. Efficient heap backing, scalable event waits, dynamic configuration, private/kernel memory accounting and supervisor recovery remain follow-ups. x2APIC, memory hotplug and NUMA remain separate. General user runtimes, PTY and compatibility execution are required before ordinary applications. The [roadmap](roadmap.md) lists terminal, Velocity and KDE checkpoints.
