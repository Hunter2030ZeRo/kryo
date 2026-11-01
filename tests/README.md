# Tests

Tests live in the packages that own the implementation:

| Location | Coverage |
| --- | --- |
| `kernel/core/tests/range.rs` | Physical ranges, alignment, and overflow |
| `kernel/core/tests/normalize.rs` | Region filtering, reservations, and a generated page oracle |
| `kernel/core/tests/pool.rs` | Frame ownership, exhaustion, reuse, initialization failure, and host-thread exclusion |
| `kernel/core/tests/vm.rs` | Private/shared mapping ownership, zeroing, capacities, rollback, permissions, and retirement |
| `kernel/core/tests/task.rs` | Dispatch, return reasons, wake/stop, resource retention, and reaping |
| `kernel/core/tests/capability.rs` | Rights, object references, duplication, close, and stale handles |
| `kernel/core/tests/object_lifetime.rs` | Forgotten strong references retain backing when the arena is dropped |
| `kernel/core/tests/capability_transfer.rs` | Reduced-rights copy delegation, receiver reservations, atomic commit, rollback, and reference lifetime |
| `kernel/core/tests/wait.rs` | Parking handoff, early/late completion, cancellation ordering, task reuse, capacity, and abandonment |
| `kernel/core/tests/deadline.rs` | Absolute times, stable selection, capacity rollback, completion retirement, and stale IDs |
| `kernel/core/tests/timeout.rs` | Object timeout, accepted delivery state, late replies, event races, and batch expiry before endpoint progress |
| `kernel/core/tests/ipc.rs` | Rendezvous, replies, cancellation, peer failure, capacity, and notification ordering |
| `kernel/core/tests/ipc_caps.rs` | Sender/capability metadata, direct and queued capacity rejection, and wait lifetime |
| `kernel/core/tests/transfer_ipc.rs` | Combined transfer/IPC acceptance, sender completion ordering, cancellation, owner death, and reservation cleanup |
| `kernel/core/tests/native_elf.rs` | Static ELF headers, bounded load plans, alignment, W^X, overlapping pages, entry validation, BSS, and unsupported formats |
| `kernel/core/tests/process_threads.rs` | Process membership, first exit, cleanup retention, generations, and last-thread termination |
| `kernel/core/tests/process_lifecycle.rs` | Durable exit state, observer races, cleanup retry, and task-slot reuse |
| `kernel/core/tests/owned_capability.rs` | Reserved publication of a newly created control object without cloning |
| `kernel/core/tests/growth.rs` | Growing tables, retained IDs/reservations, large membership/reply sets, explicit quotas, and failed growth |
| `kernel/core/tests/allocation_failure.rs` | Allocator-null failures, exact unpublished owners, strong references, partial-registration rejection, and retry |
| Core unit tests | Generation retirement and reference-count failure paths |
| HAL unit tests | Paging, contexts, kernel regions, APIC decoding, idle entry, FP save-area layout/CPU bounds, latent XCR0 validation, and FS updates |
| `interfaces/src/syscall/` tests | Version-0 call decoding and error representation |

Run the host suite with `make test`; [status](../docs/status.md) records the verified count and source evidence. Host models check structures and ordering; they do not execute privileged state transitions or prove guest behavior. Growth tests use explicit bounded constructors for quota cases and growing constructors for large tables. A separate test allocator returns null on the current host test thread to exercise real allocation-failure paths.

## Guest checks

`make smoke` enables `boot-tests`, packages the native ELF fixtures, and runs the guest checks in QEMU:

| Location | Coverage |
| --- | --- |
| `kernel/image/src/boot/checks.rs` | RAM zeroing, 128 map/unmap cycles, TLB invalidation, and frame reclamation |
| `kernel/image/src/boot/user_checks.rs` | Independent user roots, 16 isolation cases, entry-state barriers, and VM rollback |
| `kernel/image/src/boot/task_checks/` | Guarded kernel stacks, eight timer-driven switches, denied/stale capability calls, invalid saved-state isolation, and teardown |
| `kernel/image/src/boot/task_checks/ipc_checks.rs` | Eight IPC cases and three halt/wakeup cases |
| `kernel/image/src/boot/task_checks/shared_checks.rs` | Three shared pages, two delegated capabilities, acceptance ordering, and receiver-capacity rollback |
| `kernel/image/src/boot/task_checks/exec_checks.rs` | Limine module validation, native ELF mapping, two independent instances, x87/SSE/TLS isolation, optional AVX, scalar-mode restoration, and reclamation |
| `kernel/image/src/boot/task_checks/time_checks.rs` | Eleven clock/deadline scenarios, early IRQ rejection, timeout delivery states, capability lifetime, and cleanup |
| `kernel/image/src/boot/task_checks/launch_checks.rs` | Nine image rejection and unpublished VM/stack rollback checks |
| `kernel/image/src/runtime/service.rs` | Actual init/worker spawn, durable exit observation, restart, and full test teardown |
| `kernel/image/src/boot/task_checks/spawn_checks.rs` | Five real syscall rejection paths for image authority/format and handle/object/task exhaustion |
| `kernel/image/src/boot/task_checks/thread_checks.rs` | Nine real thread lifecycle, fault, join, capacity, rollback, and retirement-retry scenarios |
| `kernel/image/src/runtime/scale.rs` | Native growing registries, seven processes, 127 threads/waits, stack-branch extension, publication OOM rollback/retry, and teardown |
| `kernel/image/src/heap.rs` | Allocation alignment, run reuse, injected failure, and restored frame accounting |

The shipped runner uses standard MTTCG. Multi-CPU `boot-tests` runs the complete shared-root simultaneous-progress and CPU-migration fixture, high-target TLB retirement, missing-ACK retention, wakeup, fault/call ordering, native registry growth, and teardown. Single-CPU runs explicitly skip multi-CPU fixtures. Configured capacity and verified guest inventories are separate: host models cover all 255 identities, while the current guest matrix is recorded in [status](../docs/status.md). Temporary serialized diagnostics are preserved as failed experiments and are not shipped test modes.

The fault/call ordering fixture uses a boot-only full-batch helper so each job can finish its unchanged two-millisecond quantum without an early peer-reschedule IPI. The fault program executes UD2 without waiting on a peer barrier, and its sibling executes a real int 0x80. Exact saved RIP, terminal state, and absence of the requested debug output verify process-wide suppression of that call. Production batch scheduling and timer policy are unchanged.

IPC cases cover call/reply and plain send, reply replay, cancellation before and after acceptance, owner death with queued and accepted requests, explicit closure, and notifications signaled before or after waiting. Completion delivery writes saved return registers before publishing a task as Runnable.

Shared-memory programs create a two-page object and a one-page object, check initial zeroing, and exchange two reduced-rights capabilities. The receiver rejects a writable mapping through the read-only grant and rejects redelegation without TRANSFER. It maps the pages, closes its received handles, then reads the data after the producer closes its originals and unmaps its own pages. This includes a full-width value at the end of a page and a write through the remaining writable mapping. The fixture checks that received grants are committed before cleanup of an earlier sender completion.

A second transfer case offers two grants to a receiver with zero capability capacity. The sender receives OutOfResources, retains and closes its original handle, then sends a valid message without capabilities to the same waiting receiver. Final checks require no retained transfer batches, waits, reservations, objects, or task frames. Embedded programs live in `shared_programs.rs`.

A third case maps one delegated page RO, closes both handles, and attempts a user write. It requires #PF vector 14/error 7 at 0xc00000, task-only termination, and final reclamation of the retained backing.

Native execution loads the separate `userspace/native-smoke` static ET_EXEC module. The core load plan validates its RX/RO/RW pages; image code maps private data/BSS, four user stack pages with a low guard, and a TLS page for each instance. Startup registers are RDI = instance 41/42, RSI = TLS page, and RDX bit 0 = AVX enabled, with a 16-byte-aligned stack. The sample checks file data, zeroed BSS, and an instance-specific result before exiting with its instance ID.

The sample's assembly probe keeps distinct x87 values/control words, XMM data, MXCSR, and FS-relative sentinels across two yields and bounded busy loops. Each instance must also record actual timer preemption. SetTls call 19 installs FS; invalid bases must preserve existing TLS. The max CPU run additionally checks a YMM upper half and requires `xsave=true avx=true`. Baseline CPUs can use FXSAVE without AVX. After native tasks exit, a scalar context must still fault with #NM on `fldz`, and all task frames must be reclaimed.

Use `python3 tools/boot_smoke.py --all --cpu max` for debug and release with the XSAVE/AVX requirement. CPU selection uses separate evidence directories; see [tools](../tools/README.md). The sample uses manual FS storage, not PT_TLS. Its Rust remains scalar/soft-float and its SIMD probes are assembly; these checks do not establish general std, C ABI, dynamic linking, or service startup support.

Idle checks cover blocked consumers, a coalesced signal pending before wait, and an interrupt already pending before HLT. The BSP uses STI/HLT/CLI to close the gap between enabling interrupts and halting. This does not measure physical power use.

Timer checks also queue a one-shot interrupt with IRQs disabled and drain it through the CPL0 acknowledge-and-return path before removing the MMIO mapping.

Time tests use an ACPI-described HPET clock and a calibrated one-shot APIC countdown. They cover absolute sleep, deadline expiry before/after acceptance, notification ordering, cancellation and wait-slot reuse, receive-capacity release, capability lifetime, and an explicitly queued stale IRQ that must leave a future sleep blocked. All tasks, objects, waits, and frames are retired. Legacy idle/shutdown regressions still use raw countdown values.

Legacy guest fixtures run on the BSP before AP startup. Additional SMP fixtures and the normal init/worker path use the shared multi-CPU runtime. Objects and waits remain serialized between user execution batches. Kernel code remains soft-float. These checks do not establish general runtime support, physical scheduling latency, executable shared memory, DMA loans, move transfer, RECEIVE/WAIT owner migration, or service integration. See [status](../docs/status.md) for recorded results and evidence.

The root `tests/` subdirectories reserve future system suites. Cargo does not discover tests there automatically.

Normal boots are checked separately with `python3 tools/boot_smoke.py --normal --all` and optional `--cpu max`. The runner requires two distinct child generations, worker execution, two exit-42 observations, and a ready event sent by init. It stops its own emulator from the host; normal init is left running and no guest shutdown is claimed.

The compiled worker creates two threads in succession and joins exit codes 7/8. Main and child use separate TLS and x87/SSE/optional AVX state while sharing atomics and the handle table. All four created threads and both worker main threads must be timer-preempted. The ninth lifecycle case injects one backing-restoration failure after PTE removal and requires retained membership and failed slot reuse until explicit cleanup retry succeeds. This injection models a backend error; it is not a hardware-fault test.

Device policy adds eight host tests for checked register windows, rights, process identity, and retryable revocation. The compiled `device-probe` ELF uses mediated HPET words and real D0 notification delivery. Process stop and IRQ-consumer exit must revoke the binding before resources are reaped. The boot suite injects one cleanup failure after disabling the source, requires retained owners and denied reuse, then retries successfully. A queued D0 must drain before claim reuse. Synthetic core wait handoff combined with real HPET delivery checks that a late event cannot satisfy an already expired wait. This is not a generic PCI/DMA fence or user-MMIO mapping test.

CPU host tests cross multiword boundaries at logical slots 63/64, 127, and 254. They check directed APIC IDs 0..254, broadcast rejection, reverse lookup, execution leases, and high-slot acknowledgement. Guest SMP checks retain early AP idle, remote TLB retirement, missing-ACK retention, and collective shutdown. The configured CPU bound is 255; the final evidence matrix records which inventories actually booted. x2APIC and physical hardware remain outside these checks.

The separate `scale-probe` ELF starts six child processes; each creates twenty additional threads. The fixture requires all 127 threads to register real timed sleeps after TLS/shared-state checks, and guarded kernel stacks must cross the first 512-page leaf. It then stops and reaps all seven processes. Registry metadata preserves generations until drop, while additional stack table frames remain retained until shared-branch retirement. These are functional resource-growth checks, not scheduling throughput or power measurements.

HHDM split tables and their growing owner metadata remain kernel-owned after process teardown. Final frame checks include their exact table and heap-backing page counts, then require all other fixture allocations to return. This accounts for retained kernel ownership rather than weakening reclamation checks.

RAM metadata uses boot-reserved external bitmap/summary buffers. Pool host tests
model exactly 128 GiB and a larger sparse plan without backing all payload RAM;
undersized buffers, retry, one-time publication, tail allocation, and protected
reservations are checked. Guest tests allocate real owned pages above 4 GiB and
128 GiB when available, compare HHDM/scratch aliases, zero and reuse them, and
require exact accounting. Remote-TLB and missing-ACK tests prefer high physical
backing. The low-memory VM fixture explicitly selects [0, 2 MiB) instead of
assuming the global allocation cursor moves in address order.

The final RAM matrix and source manifest are under
`docs/validation/2026-11-01-ram/`. Requested guest RAM, actually managed Usable
frames, sparse-page access, and host resident memory are separate observations.
