# Implementation status

Snapshot: 2026-10-31. The [design contract](design-contract.md) defines requirements; this file records implemented behavior.

## Memory and boot

- Limine memory/HHDM/image/framebuffer requests, four-level paging, and a 256 KiB bootstrap stack.
- GDT/TSS/IST and IDT setup, emergency diagnostics, breakpoint return, debug output, and a 32bpp framebuffer writer.
- Checked physical ranges, reservation subtraction, and a one-time bitmap frame pool with allocation tokens.
- HHDM access, zeroing, and BSP scratch mappings.
- Bounded user spaces owning their root, intermediate tables, and data frames.
- RO/RW/RX mappings, initialization before publication, inactive-space mutation, and explicit cleanup.
- Supervisor-only kernel mappings, HHDM NX policy, and read-only aliases for executable/read-only user frames.
- Huge HHDM splitting with retained kernel table ownership.
- A supervisor-only kernel region for uncached APIC access and guarded per-task entry stacks.

Core remains HAL-independent through the unsafe VmBackend contract. Image code connects the concrete HAL to core.

## Execution and authority

The BSP can resume scalar Ring 3 contexts in separate user spaces. All general registers, RIP/RSP/RFLAGS, and FS/GS bases are saved. Each test task owns a 16 KiB kernel entry stack with an unmapped lower guard. The HAL updates TSS.RSP0 for entry and restores kernel execution state before returning to the scheduler.

A local xAPIC timer preempts user execution. A fixed-capacity round-robin task table tracks one Running task and retains Dying resources until explicit cleanup succeeds. Invalid saved user state and fatal user exceptions stop the affected task.

The object arena retains strong references. Process-local handles validate generation, kind, and rights; close releases one reference and duplicate only preserves or reduces authority. Experimental int 0x80 calls provide yield, exit, task status/stop, close, and duplicate alongside the IPC operations below.

Bounded endpoint IPC adds Send/Receive/Call/Reply with 32 inline bytes, single-use reply tokens, cancellation, closure, and peer-death cleanup. Notifications coalesce signals. Generational waits preserve completions during Parking and publish saved return values before waking a task. SEND/RECEIVE/WAIT/SIGNAL rights are checked at entry.

When all test tasks block, the BSP uses `sti; hlt; cli` and returns on an interrupt. The timer handler records delivery without taking normal kernel locks; deferred image code signals the waiting object. This is an idle mechanism, not a measurement of power use.

These paths are connected by `boot-tests`. Normal boots do not yet load or schedule services. [ADR 0002](decisions/0002-user-address-spaces.md) describes memory ownership, [ADR 0003](decisions/0003-preemption-and-capabilities.md) task execution, and [ADR 0004](decisions/0004-waits-ipc-and-idle.md) waits, IPC, and idle handoff.

## Verification

Latest [evidence](validation/2026-10-31-ipc/evidence.json) and logs:
[debug](validation/2026-10-31-ipc/debug.log),
[release](validation/2026-10-31-ipc/release.log).

| Check | Result |
| --- | --- |
| no_std workspace check and Rust formatting | Passed |
| Workspace Clippy, all features, warnings denied | Passed |
| Host tests | 114 passed: core 92, HAL 21, interfaces 1 |
| Core task/object/capability tests | 23 tests cover ownership, cleanup retry, exhaustion, stale generations, slot retirement, reference limits, and rights |
| Existing VM failure models | Allocation/publication failures, alias quarantine, inactive-state checks, and retirement ordering passed |
| QEMU TCG debug/release | All boot markers passed in each profile |
| User memory and entry regressions | 16 prior user cases and 128 mapping cycles passed |
| Noncooperative preemption | Two infinite-loop tasks each received four slices without a yield; caller/callee register and memory counters progressed |
| Capability calls | Three forbidden operations and one closed-handle lookup rejected; reduced duplication, close, yield, target stop, and exit completed |
| Invalid saved context | A task set RSP=0 and looped; resume rejected it while its peer continued |
| Pending timer shutdown | A one-shot timer queued vector 0xe0 with IRQs off; shutdown drained it before MMIO removal |
| Task reclamation | User mappings, entry stacks, objects/handles, and region tables reclaimed; task-stage frame count restored |
| Wait lifetime | 10 new host tests cover early/late completion, first-result wins, ownership, exhaustion, reuse, abandonment, and generation retirement |
| Endpoint/notification models | 17 new host tests cover rendezvous, FIFO, bounds, reply identity, cancellation, owner/caller death, closure, and coalescing |
| Guest IPC | Eight scenarios passed, including 32-byte request/reply, Send, denied Receive, reply replay, queued/accepted cancellation, closure, and owner death |
| Guest idle | Two notification cases blocked all remaining tasks, halted, then woke on an IRQ and resumed; a pending-before-HLT case also passed |
| Kernel continuation | Existing user-stage accounting, framebuffer path, and final ABC completed |

The full memory/user sequence retains one explicitly owned HHDM split-table frame. The new task stage returns every allocation it adds. Table permissions and guard presence are checked by the guest fixtures.

An initial task run timed out after the capability checks while a count-1 periodic timer was used to force pending delivery. The final regression uses a bounded one-shot timer and passes both profiles. The [earlier log](validation/2026-10-31-tasks/prior-timeout.log) is retained; the exact cause of that timeout was not established.

Historical records remain available for [task execution](validation/2026-10-31-tasks/evidence.json), [user entry](validation/2026-10-31-userspace/evidence.json), [memory bootstrap](validation/2026-10-31-memory/evidence.json), and the [layout migration](validation/2026-10-30-layout-notes.md). Host tests, QEMU execution, and hardware measurements are separate evidence. The runner stops QEMU from the host after the final marker; it does not verify guest shutdown.

## Limits

- BSP only, four-level paging, PCID off, and exclusive page-table access. Kernel maskable IRQs remain disabled except controlled idle and shutdown-drain windows; user execution enables them for the local timer.
- xAPIC only. Timer counts are uncalibrated and do not establish a millisecond quantum, timeout clock, or performance result.
- Scalar user contexts only. FP/SIMD remains trapped; general runtime, TLS setup, and broader CPU-state support are pending.
- Guest fixture capacities: two tasks/waits, eight objects, eight handles per task, two queued senders and two accepted replies per endpoint. One task owns RECEIVE and one owns each notification's WAIT. There is no general process/thread hierarchy or dynamic service startup.
- Static boot pool: 288 ranges and 4 GiB of managed RAM. User spaces: 16 mappings and 64 intermediate tables each.
- Initial mappings have unique ownership. Shared memory, pin accounting, quotas, and revocation are pending.
- HHDM split tables remain kernel-owned, with a 32-frame boot budget and no coalescing. Bootloader-reclaimable/ACPI memory and active boot tables remain reserved.
- Kernel heap, stable calls, timed/multi-object waits, capability transfer, DMA isolation, and SMP remain pending. IPC currently carries inline data only. Wait/object changes require serialized ownership of one persistent wait table; SMP synchronization is not implemented.
- Idle tests use a one-shot count. Clock calibration, deadline queues, device IRQ routing, NMI work scheduling, general tickless/C-state policy, and physical power measurements remain pending.
- ICE execution, BetAFS I/O, terminal/Velocity, and KDE integration remain unimplemented.

## Next work

Add shared-memory objects and bounded capability transfer with receiver capacity reserved before acceptance. Preserve accepted resource lifetimes through cancellation and peer death. Calibrated time and FP/SIMD support remain prerequisites for general applications; service startup follows the required image/runtime integration.
