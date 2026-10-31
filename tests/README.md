# Tests

Tests live in the packages that own the implementation:

| Location | Coverage |
| --- | --- |
| `kernel/core/tests/range.rs` | Physical ranges, alignment, and overflow |
| `kernel/core/tests/normalize.rs` | Region filtering, reservations, and a generated page oracle |
| `kernel/core/tests/pool.rs` | Frame ownership, exhaustion, reuse, initialization failure, and host-thread exclusion |
| `kernel/core/tests/vm.rs` | Mapping ownership, capacities, rollback, permissions, and retirement |
| `kernel/core/tests/task.rs` | Dispatch, return reasons, wake/stop, resource retention, and reaping |
| `kernel/core/tests/capability.rs` | Rights, object references, duplication, close, and stale handles |
| `kernel/core/tests/wait.rs` | Parking handoff, early/late completion, cancellation ordering, task reuse, capacity, and abandonment |
| `kernel/core/tests/ipc.rs` | Rendezvous, replies, cancellation, peer failure, capacity, and notification ordering |
| Core unit tests | Generation retirement and reference-count failure paths |
| HAL unit tests | Paging, contexts, kernel regions, APIC decoding, and idle-entry validation |
| `interfaces/src/syscall/` tests | Version-0 call decoding and error representation |

The suite contains 114 host tests: core 92, HAL 21, and interfaces 1. Run it with `make test`. Waits account for 10 tests and IPC for 17, including generation-retirement unit cases.

## Guest checks

`make smoke` enables `boot-tests` and runs the embedded guest checks in QEMU:

| Location | Coverage |
| --- | --- |
| `kernel/image/src/boot/checks.rs` | RAM zeroing, 128 map/unmap cycles, TLB invalidation, and frame reclamation |
| `kernel/image/src/boot/user_checks.rs` | Independent user roots, 16 isolation cases, entry-state barriers, and VM rollback |
| `kernel/image/src/boot/task_checks/` | Guarded kernel stacks, eight timer-driven switches, denied/stale capability calls, invalid saved-state isolation, and teardown |
| `kernel/image/src/boot/task_checks/ipc_checks.rs` | Eight IPC cases and three halt/wakeup cases |

IPC cases cover call/reply and plain send, reply replay, cancellation before and after acceptance, owner death with queued and accepted requests, explicit closure, and notifications signaled before or after waiting. Completion delivery writes saved return registers before publishing a task as Runnable.

Idle checks cover blocked consumers, a coalesced signal pending before wait, and an interrupt already pending before HLT. The BSP uses STI/HLT/CLI to close the gap between enabling interrupts and halting. This does not measure physical power use.

Timer checks also queue a one-shot interrupt with IRQs disabled and drain it through the CPL0 acknowledge-and-return path before removing the MMIO mapping.

Guest tests use one BSP, serialized objects/waits, scalar code, and fixed LAPIC counter values. They do not establish SMP behavior, general FP/SIMD runtime support, calibrated scheduling intervals, IPC capability transfer, or service integration. See [status](../docs/status.md) for recorded results and evidence.

The root `tests/` subdirectories reserve future system suites. Cargo does not discover tests there automatically.
