# Repository layout

The kernel workspace contains four packages. `userspace/` has a separate workspace for native libraries, supervisor/service programs and regression executables; native-smoke remains its own executable workspace. Reserved directories use `.gitkeep`; add modules and packages when their implementation exists.

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
| `hal/src/arch/x86_64/context/` | Resumable Ring 3 contexts, scalar and extended-state entry, calls, faults, timer entry, and validated FS base updates |
| `hal/src/arch/x86_64/context/extended.rs` | Private aligned x87/SSE state, bounded optional AVX state, CPU validation, and kernel-state preservation |
| `hal/src/arch/x86_64/apic/` | BSP xAPIC timer, EOI, pending-interrupt drain, idle halt/wakeup, and configuration restoration |
| `hal/src/arch/x86_64/acpi.rs` | Bounded RSDP/root/HPET discovery with checksums and address validation |
| `hal/src/arch/x86_64/clock.rs` | Retained 64-bit HPET monotonic clock and explicit hardware restoration |
| `hal/src/arch/x86_64/paging/` | HHDM protection, owned user tables, scratch mapping, and shared kernel regions for guarded stacks/MMIO |
| `hal/src/debug/console.rs` | Early formatted and emergency output |
| `hal/src/io/framebuffer.rs` | Checked 32bpp framebuffer writes |
| `kernel/core/src/mm/frame/` | Ranges, normalization, frame ownership, and allocation |
| `kernel/core/src/mm/shared/` | Bounded shared-memory backing, one-time initialization, and strong page-lease contract |
| `kernel/core/src/mm/vm/` | Private and shared mapping ownership, permissions, rollback, and retirement |
| `kernel/core/src/task/` | Bounded task table, round-robin dispatch, state transitions, resource retention, and reaping |
| `kernel/core/src/sched/` | Task states and scheduler return reasons |
| `kernel/core/src/exec/` | Borrowed static ELF64 load plans, permission/range validation, and page initialization |
| `kernel/core/src/process/` | Generational process ownership/membership and durable exit observers |
| `kernel/core/src/object/` | Generational object arena and strong references |
| `kernel/core/src/capability/` | Process-local handles, rights checks, duplication, close, and stale-handle rejection |
| `kernel/core/src/capability/transfer.rs` | Reduced-rights `TransferBatch` ownership and receiver `HandleReservation` commit/rollback |
| `kernel/core/src/sync/wait.rs` | Bounded generational waits, Parking handoff, single completion, and teardown |
| `kernel/core/src/ipc/` | Bounded endpoint rendezvous, capability counts and receiver capacity, call/reply tokens, cancellation, and notifications |
| `interfaces/src/syscall/` | Experimental version-0 bootstrap calls and results |
| `kernel/core/tests/` | Host tests for memory, tasks, capabilities, waits, IPC, transfers, and native ELF plans |
| `kernel/image/src/main.rs` | Entry, panic handling, framebuffer exercise, and breakpoint return |
| `kernel/image/src/runtime/mod.rs` | Process/thread resource definitions, CPU handoff, and deadline idle |
| `kernel/image/src/runtime/tasks.rs` | Independent process registry and thread scheduler, membership, group exit, and retirement |
| `kernel/image/src/runtime/threads.rs` | Managed thread-stack/TLS preparation, control publication, joins, and retained rollback |
| `kernel/image/src/runtime/native.rs` | Shared normal/test native call dispatcher |
| `kernel/image/src/runtime/stacks.rs` | Fallible guarded stack construction and retained cleanup retry |
| `kernel/image/src/runtime/launch.rs` | Private ELF preparation and retained unpublished owners |
| `kernel/image/src/runtime/process.rs` | Spawn transaction, reserved result handle, and process observation registration |
| `kernel/image/src/runtime/service.rs` | Normal init startup and the shared supervision guest check |
| `kernel/image/src/boot.rs` | Limine requests, including boot-test ModulesRequest and bounded native module selection |
| `kernel/image/src/boot/memory.rs` | Boot memory interpretation and frame-pool activation |
| `kernel/image/src/boot/framebuffer.rs` | Limine framebuffer conversion |
| `kernel/image/src/boot/checks.rs` | Guest memory tests |
| `kernel/image/src/boot/user_checks.rs` | Separate user roots, fault isolation, permission checks, and VM reclamation |
| `kernel/image/src/boot/vm_backend.rs` | Core/HAL VM adapter, shared-frame initialization, and persistent kernel-table ownership |
| `kernel/image/src/boot/task_checks/` | Task execution, guarded stacks, bootstrap call dispatch, preemption, and teardown checks |
| `kernel/image/src/runtime/ipc_runtime.rs` | Staged transfers, exit observations, receiver grant commit, completion delivery, wakeup, and dead-wait cleanup |
| `kernel/image/src/boot/task_checks/ipc_checks.rs` | Guest rendezvous, cancellation, peer failure, notifications, and idle checks |
| `kernel/image/src/boot/task_checks/ipc_programs.rs` | Embedded scalar IPC test programs |
| `kernel/image/src/boot/task_checks/shared_checks.rs` | Guest shared-memory lifetime, transfer acceptance ordering, and receiver-capacity rollback checks |
| `kernel/image/src/boot/task_checks/shared_programs.rs` | Embedded shared-memory producer, receiver, and transfer-capacity programs |
| `kernel/image/src/boot/task_checks/exec_checks.rs` | Native ELF mapping, private startup memory, extended task contexts, execution, and reclamation checks |
| `kernel/image/src/runtime/clock.rs` | Firmware mapping/provenance, clock initialization, and local timer calibration |
| `kernel/image/src/boot/task_checks/time_checks.rs` | Deadline, timeout, cancellation, stale IRQ, and cleanup checks |
| `kernel/image/src/boot/task_checks/time_programs.rs` | Ring 3 clock, sleep, and timed IPC programs |
| `kernel/image/build.rs` | Linker arguments for the kernel binary |
| `userspace/native-smoke/` | Separate static native ELF with Rust data/BSS checks and assembly x87/SSE/AVX/TLS probes |
| `tools/build_user.py` | Isolated user-code build flags and native ELF output |
| `tools/boot_smoke.py` | Bounded headless QEMU regression runner |

Guest regression checks are gated by `boot-tests`. Embedded byte programs retain the scalar entry path. Native execution checks load a separately compiled ELF module into two independent address spaces and give each task its own `ExtendedState`. Both paths use one CPU.

`ElfImage` validates static little-endian AMD64 ET_EXEC files without allocating or mapping memory. It rejects dynamic linking and PT_TLS, requires a file-backed executable entry, enforces W^X, and rejects load segments that share a rounded page. The image package initializes private pages, maps the plan, and supplies a guarded user stack and private TLS page before making a task runnable.

`ExtendedState` owns a private 1024-byte area aligned to 64 bytes. HAL selects FXSAVE for x87/SSE or standard XSAVE for x87/SSE and optional AVX within that bound. It saves user state before Rust trap handling and restores the original kernel FP state and CPU controls before returning. Existing scalar contexts still trap FP use. Kernel Rust remains soft-float; this mechanism does not permit arbitrary kernel SIMD use.

## Reserved kernel work

HAL's x86-64 `boot/`, `syscall/`, `smp/`, `iommu/`, and `asm/` directories reserve additional CPU-entry support, a general syscall path, AP startup, DMA isolation, and assembly support. The implemented `cpu.rs` owns the 64-slot APIC registry; `smp.rs` owns execution leases and IPI/TLB epochs. `hpet_device.rs` owns the mediated comparator. Optional FRED remains unimplemented.

`SharedMemory` owns a bounded array of frames. Each shared mapping keeps a `SharedPage` lease that strongly retains the memory object after its handles close. Unmap retires the user PTE and TLB access before dropping that lease; it leaves the shared RW+NX kernel alias unchanged. Backing returns to the allocator only after the last owner is released. Unclosed address spaces and forgotten object references preserve ownership rather than free live storage.

VM work still includes pager/fault policy and more scalable mapping coordination. SMP mapping lifetime uses acknowledged worker quiescence and global TLB epochs. Shared pages support RO/RW user mappings and cannot be executable. The `mm/heap/` and `mm/dma/` modules remain reserved; DMA loans are not implemented.

Core's `init/`, `interrupt/`, `syscall/`, `time/`, `security/`, and `diagnostics/` directories remain reserved. Absolute deadlines live in existing `sync/wait.rs` slots; they do not require a separate time subsystem yet. HAL calibrates one-shot APIC events against HPET. Synchronization currently provides serialized wait state, not a general SMP locking model.

Objects provide identity and lifetime; capabilities provide authority. Bounded capability transfer copies strong references with reduced rights; source handles remain valid. Move transfer, RECEIVE/WAIT owner migration, and service protocols remain pending. VFS and NT-specific object policy remain outside core.

## Interfaces

Version 0 defines bootstrap task and handle calls 0-5. Calls 6-13 add Send, Receive, Call, Reply, EndpointClose, Wait, Signal, and CancelWait. Calls 14-18 add MemoryCreate, MemoryMap, MemoryUnmap, SendCaps, and CallCaps. Call 19, SetTls, sets the calling context's FS base to zero or a lower-half user address; an invalid base leaves it unchanged.

Calls 20-27 add ClockRead, SleepUntil, and six timed IPC variants. Deadlines are absolute monotonic nanoseconds, and timeout preserves delivery state. The image detaches all due waits before allowing endpoint progress, then commits grants and publishes saved results before waking tasks.

IPC carries four u64 words (32 bytes) and up to two delegated capabilities. The receiver reserves handle slots before waiting. Endpoint completion metadata records the sender, sender wait, and capability count; the image commits grants before discarding sender completions or waking tasks. Receive returns an endpoint-local reply token and destination handles. Shared-memory calls map one object page at a time with RO/RW permissions.

Calls 28-31 add image-capability Spawn, retained ProcessWait, capability-gated inline DebugLog, and ProcessWaitUntil. Calls 32-35 add managed ThreadCreate, ThreadExit, ThreadJoin, and ThreadJoinUntil. The image supplies dispatch and completion delivery during normal boot and checks. Objects and waits are serialized on the BSP. This remains an experimental interface.

The `handle/`, `ipc/`, `memory/`, `device/`, `filesystem/`, and `service/` interface directories remain reserved.

## User space

| Path under userspace/ | Responsibility |
| --- | --- |
| `libsys/` | Native call wrappers, checked results and immutable startup decoding |
| `services/` | Role allowlists, service lifecycle and discovery policy |
| `runtime/` | Reserved general process startup/runtime support |
| `bootstrap-worker/` | Native process with successive OS threads, shared data/handles, and private TLS/FP probes |
| `native-smoke/` | Standalone native ELF fixture, startup convention, and CPU-state probes |
| `init/` | Implemented first process and bounded user-space supervision |
| `supervisor/` | Configured service startup, private discovery, monitoring and generations; general state handoff pending |
| `echo-service/` | Native request/reply service and failure-boundary fixture |
| `registry-client/` | Discovery, reduced authority and denied-request fixtures |
| `device-manager/` | Device discovery, driver matching, and authority distribution |
| `vfs/` | Namespaces, mounts, file handles, and filesystem routing |
| `pager/` | File-backed page supply and cache policy |
| `net/` | Network stack and socket service |
| `drivers/` | Serial, VirtIO, NVMe, input, display, and network services |
| `filesystems/tmpfs/` | In-memory filesystem |
| `filesystems/betafs/` | Adapter for the independent BetAFS engine |

`make user` builds ten executables and verifies their stripped boot copies against full Cargo load images. Normal boot validates the device and legacy channel probes, then starts `kryo-supervisor` with an explicit immutable startup inventory. It launches the echo service and registry clients through manifests. Boot-tests additionally run startup failure fixtures, the earlier init/worker/thread regressions and native-smoke/growth checks. General device management and user runtime services remain pending.

Its startup convention supplies the task instance in RDI, a private TLS page in RSI, and an AVX-enabled flag in RDX bit 0, with a 16-byte-aligned stack. The program installs FS through SetTls and uses manual FS-relative storage; ELF TLS templates are not loaded. Rust code uses the soft-float bare-metal target, while assembly probes x87/SSE and optional AVX across yields and timer preemption. This static executable uses no dynamic libraries and does not establish general std or C ABI support.

## Compatibility and tests

ICE's `linux/` and `nt/` paths independently adapt their ABIs to native facilities. `compat/ice/userspace/` reserves loader and runtime integration.

The root test directories reserve boot, syscall, IPC, recovery, compatibility, performance, and unit scenarios. Current host tests live in core, HAL, and interfaces; guest tests are driven by the smoke tool. See [tests](../tests/README.md).

`tools/xtask/` and `tools/debug/` remain reserved. Current build and run commands are defined in the Makefile.
