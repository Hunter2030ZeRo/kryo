# Repository layout

The workspace contains four packages. Reserved directories use `.gitkeep`; add modules and packages when their implementation exists.

## Packages and boot inputs

| Path | Responsibility |
| --- | --- |
| `boot/` | Linker script, Limine configuration, and initial-image inputs |
| `hal/` | Architecture-specific mechanisms and hardware access |
| `interfaces/` | Native call and service protocol representations |
| `kernel/core/` | Resource ownership, execution, authority, and communication |
| `kernel/image/` | Boot interpretation, component wiring, entry, and panic handling |
| `userspace/` | Ring 3 services, drivers, and client libraries |
| `compat/ice/` | Independent Linux and NT compatibility adapters |
| `tests/` | Reserved system and integration suites |
| `tools/` | Build, boot, and debugging tools |
| `docs/` | Design, decisions, status, and validation records |

## Active source

| Path | Implementation |
| --- | --- |
| `hal/src/arch/x86_64/gdt.rs` | Kernel/user descriptors, CS/SS, and TR loading |
| `hal/src/arch/x86_64/tss.rs` | BSP TSS, current kernel entry stack, and double-fault stack |
| `hal/src/arch/x86_64/interrupts/idt.rs` | Exception and interrupt gate installation |
| `hal/src/arch/x86_64/context/` | Resumable scalar Ring 3 contexts, register save/restore, calls, faults, and timer entry |
| `hal/src/arch/x86_64/apic/` | BSP xAPIC timer, EOI, pending-interrupt drain, idle halt/wakeup, and configuration restoration |
| `hal/src/arch/x86_64/paging/` | HHDM protection, owned user tables, scratch mapping, and shared kernel regions for guarded stacks/MMIO |
| `hal/src/debug/console.rs` | Early formatted and emergency output |
| `hal/src/io/framebuffer.rs` | Checked 32bpp framebuffer writes |
| `kernel/core/src/mm/frame/` | Ranges, normalization, frame ownership, and allocation |
| `kernel/core/src/mm/vm/` | Owned address spaces, mapping lifetime, permissions, and backend contract |
| `kernel/core/src/task/` | Bounded task table, round-robin dispatch, state transitions, resource retention, and reaping |
| `kernel/core/src/sched/` | Task states and scheduler return reasons |
| `kernel/core/src/object/` | Generational object arena and strong references |
| `kernel/core/src/capability/` | Process-local handles, rights checks, restricted duplication, close, and stale-handle rejection |
| `kernel/core/src/sync/wait.rs` | Bounded generational waits, Parking handoff, single completion, and teardown |
| `kernel/core/src/ipc/` | Bounded endpoint rendezvous, call/reply tokens, cancellation, and coalescing notifications |
| `interfaces/src/syscall/` | Experimental version-0 bootstrap calls and results |
| `kernel/core/tests/` | Host tests for memory, tasks, capabilities, waits, and IPC |
| `kernel/image/src/main.rs` | Entry, panic handling, framebuffer exercise, and breakpoint return |
| `kernel/image/src/boot.rs` | Limine requests and boot module wiring |
| `kernel/image/src/boot/memory.rs` | Boot memory interpretation and frame-pool activation |
| `kernel/image/src/boot/framebuffer.rs` | Limine framebuffer conversion |
| `kernel/image/src/boot/checks.rs` | Guest memory tests |
| `kernel/image/src/boot/user_checks.rs` | Separate user roots, fault isolation, permission checks, and VM reclamation |
| `kernel/image/src/boot/vm_backend.rs` | Core/HAL VM adapter and persistent kernel-table ownership |
| `kernel/image/src/boot/task_checks/` | Task execution, guarded stacks, bootstrap call dispatch, preemption, and teardown checks |
| `kernel/image/src/boot/task_checks/ipc_runtime.rs` | Completion delivery, task wakeup, channel detachment, and dead-wait cleanup |
| `kernel/image/src/boot/task_checks/ipc_checks.rs` | Guest rendezvous, cancellation, peer failure, notifications, and idle checks |
| `kernel/image/src/boot/task_checks/ipc_programs.rs` | Embedded scalar IPC test programs |
| `kernel/image/build.rs` | Linker arguments for the kernel binary |
| `tools/boot_smoke.py` | Bounded headless QEMU regression runner |

Guest regression checks are gated by `boot-tests`. Their embedded programs exercise scalar execution on one CPU; they are not user-space service packages.

## Reserved kernel work

HAL's x86-64 `boot/`, `syscall/`, `smp/`, `iommu/`, and `asm/` directories reserve additional CPU-entry support, a general syscall path, AP startup, DMA isolation, and assembly support. The existing `cpu.rs`, `interrupts/traps.rs`, and `interrupts/fred.rs` files remain placeholders.

VM work still includes shared memory references, pager/fault policy, and SMP mapping lifetime. Other reserved memory modules are `mm/heap/`, `mm/shared/`, and `mm/dma/`.

Core's `init/`, `interrupt/`, `syscall/`, `time/`, `security/`, and `diagnostics/` directories remain reserved. Synchronization currently provides serialized wait state, not a general SMP locking model. The xAPIC timer uses fixed counter values; calibrated time services and wait deadlines are not implemented.

Objects provide identity and lifetime; capabilities provide authority. VFS and NT-specific object policy remain outside core. Shared memory, IPC capability transfer, and service protocols remain pending.

## Interfaces

Version 0 defines bootstrap task and handle calls 0-5. Calls 6-13 add Send, Receive, Call, Reply, EndpointClose, Wait, Signal, and CancelWait. IPC carries four u64 words (32 bytes); receive also returns an endpoint-local reply token. Capability transfer is not implemented.

The image supplies dispatch and completion delivery. Objects and waits are serialized on the BSP. This is an experimental interface, not a stable service ABI.

The `handle/`, `ipc/`, `memory/`, `device/`, `filesystem/`, and `service/` interface directories remain reserved.

## User space

| Path under userspace/ | Responsibility |
| --- | --- |
| `libsys/`, `runtime/` | Native call wrappers and process startup/runtime support |
| `init/` | First process and initial capability distribution |
| `supervisor/` | Service monitoring, restart, state handoff, and generation management |
| `device-manager/` | Device discovery, driver matching, and authority distribution |
| `vfs/` | Namespaces, mounts, file handles, and filesystem routing |
| `pager/` | File-backed page supply and cache policy |
| `net/` | Network stack and socket service |
| `drivers/` | Serial, VirtIO, NVMe, input, display, and network services |
| `filesystems/tmpfs/` | In-memory filesystem |
| `filesystems/betafs/` | Adapter for the independent BetAFS engine |

These service directories remain reserved. No service executable or runtime package is built yet.

## Compatibility and tests

ICE's `linux/` and `nt/` paths independently adapt their ABIs to native facilities. `compat/ice/userspace/` reserves loader and runtime integration.

The root test directories reserve boot, syscall, IPC, recovery, compatibility, performance, and unit scenarios. Current host tests live in core, HAL, and interfaces; guest tests are driven by the smoke tool. See [tests](../tests/README.md).

`tools/xtask/` and `tools/debug/` remain reserved. Current build and run commands are defined in the Makefile.
