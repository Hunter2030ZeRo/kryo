# Architecture

Kryo is the microkernel of Cryolite. It uses Rust `no_std`, targets x86-64, and boots through Limine.

This document defines subsystem boundaries. The [design contract](design-contract.md) specifies ownership and synchronization rules. [Status](status.md) records implemented behavior.

## Kernel boundary

Ring 0 provides CPU control, memory management, execution, capabilities, and communication mechanisms. General device drivers, VFS, filesystems, and networking run as separate Ring 3 services.

Interrupt controllers, the MMU, and early debug output remain kernel mechanisms. Service discovery, driver selection, filesystem routing, and restart policy belong in user space.

The design targets low overhead and fault isolation. Small requests use kernel IPC; bulk transfers use registered shared memory. Performance decisions require measurements of copying, scheduling, and wakeup costs.

The initial IPC path carries four u64 words through synchronous endpoints. Wait completion is separate from scheduler publication: a completion during Parking remains hidden until execution handoff finishes. Object code records results; image wiring writes the saved return context and then wakes the task after object access ends. Coalescing notifications retain early signals.

IPC may also delegate up to two reduced-rights capabilities. The receiver reserves handle capacity before waiting; accepted grants install before sender cleanup or receiver wakeup. Shared-memory mappings retain the backing object independently of handles. Initial shared pages are non-executable and support RO/RW views.

Native services can create endpoints and notifications through an explicitly granted channel factory. Factories share a finite live-payload account through duplication, delegation and process/thread creation. The creating thread owns receiving or waiting. Registry references retire after accepted transfers and completions settle; retained closed objects stay charged until final destruction. [ADR 0014](decisions/0014-native-channel-creation.md) defines creation and retirement.

When no task is runnable, the BSP arms a one-shot local timer for the earliest pending deadline, enables interrupts, and halts in one instruction sequence. It returns with interrupts disabled. IRQ handlers record events; deferred kernel work samples the monotonic clock before completing waits. A 64-bit ACPI-described HPET supplies elapsed nanoseconds and calibrates the APIC countdown. User slices end at the earlier of two milliseconds and the next wait deadline. Stale or early timer IRQs trigger another time check. Hardware latency and power measurements remain separate work.

## Packages

| Package | Responsibility |
| --- | --- |
| `hal` | CPU entry, registers, exception gates, page tables, TLB operations, and hardware access |
| `kernel/core` | Resource ownership, address spaces, tasks, kernel objects, capabilities, IPC, and scheduling |
| `kernel/image` | Limine responses, component initialization, kernel entry, and panic handling |
| `interfaces` | Versioned user-visible representations and protocols |
| `userspace/libsys` | Native call wrappers and immutable startup-record decoding |
| `userspace/services` | Role allowlists, service generations, discovery and lifecycle policy |

Current internal Cargo dependencies:

```text
kernel/image --> kernel/core
             --> hal
             --> interfaces
```

Core uses `alloc` for fallible metadata growth and `spin` for synchronization; it has no HAL or interfaces dependency. Hardware operations are connected by the image package. Future core-to-HAL interfaces must preserve this separation and must not introduce a HAL-to-core dependency cycle. Prefer static dispatch where it keeps the interface simple.

User programs call the native ABI through their runtime or `libsys`. They do not link privileged core or HAL code.

Core now validates a bounded static ELF image into a borrowed page plan; image code copies those pages into owned RO/RW/RX mappings. Limine supplies the initial executable bytes as a retained module. The separate native test package has no dependency on privileged kernel packages. Extended user contexts own private FP/SIMD save areas; the kernel remains compiled without FP/SIMD use.

A growing process registry owns each ProcessResources address space and handle table. A separate growing scheduler owns ThreadResources CPU state, FP/SIMD storage, guarded kernel stacks, execution-memory leases and waits. Process and thread IDs have independent generations. Process-control and thread-control objects retain terminal results independently of execution storage. Process resources remain retained until the final member's cleanup succeeds. [ADR 0009](decisions/0009-bsp-user-threads.md) defines the lifecycle; [ADR 0012](decisions/0012-growing-registries.md) defines metadata growth and failure.

ThreadCreateNative borrows runtime-provided private RW stack/TLS spans and an
optional unmapped guard. The kernel validates and leases those ranges before
publication, installs FS before user entry and creates only the kernel entry
stack and CPU state. Core lease checks block conflicting mapping changes before
invalidation. Admitted conflicts return Busy; legacy reserved-range exclusions
retain InvalidArgument. Actual execution handoff releases the lease before terminal join
publication; user mappings remain until their runtime removes them. Legacy
ThreadCreate still supplies managed user storage. This ownership boundary is
defined in [ADR 0027](decisions/0027-runtime-owned-thread-memory.md).

The SMP runtime supports up to 255 xAPIC logical processors. Directed APIC IDs are 0..254; 255 is the broadcast destination. CPU sets span multiple 64-bit words, so a machine-word boundary does not limit ownership or retirement masks. Each owns its GDT/TSS, entry state, local timer, and kernel FP area. The BSP dispatches owned context payloads to AP mailboxes and may execute a job itself. Threads can run concurrently in one process root. Before native calls, VM changes, or cleanup, all workers save state and acknowledge their boot-CR3 reload. Core policy remains serialized on the BSP. Kernel mapping changes additionally require acknowledged global TLB invalidation. [ADR 0010](decisions/0010-smp-execution.md) defines these ownership and retirement rules.

Core wait slots retain optional absolute deadlines in their existing records. Registry growth can allocate metadata; registering another deadline does not allocate a separate timer object. Timeout detaches every registration due at the sampled time before endpoint progress can accept more requests. The same first-completion and delivery-state rules apply to events, timeout, cancellation, closure, and task death. Image code owns timer-only sleep records and writes results before runnable publication. The wait table uses a linear deadline scan; the HAL owns clock hardware and countdown conversion.

## Metadata and memory allocation

The image sizes frame-allocation bitmaps from the boot memory map before enabling the heap. It carves a page-aligned span from normalized Usable RAM, reserves the entire span, and normalizes the remaining ranges again. The frame pool retains exclusive HHDM-backed buffers for its lifetime. Small-memory boots allocate small metadata; larger RAM inventories do not require a larger kernel BSS array. [ADR 0013](decisions/0013-runtime-frame-metadata.md) defines this bootstrap ownership and its limits.

Normal service execution grows process, thread, membership, object, handle, wait, observation, and VM metadata as needed. Explicit inline constructors retain bounded storage for exhaustion fixtures. Registry growth uses fallible reservation; creation errors return unpublished owners and incomplete cleanup stays reachable. Generational slot histories remain until their registry retires. Moving metadata never moves CPU-local GDT/TSS storage, changes IDs, or transfers an active execution owner.

Production processes now bind to boot-minted resource accounts before startup.
Handles, active waits, live threads and shared-memory payload pages retain charges
through actual retirement. Ordinary children inherit their parent's account;
selecting another requires ASSIGN and TRANSFER. Per-process VM mapping/table
limits also constrain aliases. These are service policy bounds, not global
process/thread limits or exact physical-RAM accounting. CPU-time budgets and
kernel-metadata byte accounting remain pending. [ADR 0016](decisions/0016-service-resource-budgets.md)
defines charge ownership and the scheduler/SIMD boundary.

The image's object heap uses pinned upstream Talc over the existing page
allocator. Small objects share arenas instead of owning whole page runs. The
backing source retains each original frame-run token in a private prefix and
returns empty secondary arenas; the first metadata arena remains retained.
IRQ exclusion precedes the shared heap mutex, and backing operations use only
the allocation-free frame pool. Global live-object bytes/counts are separate
from backing-page accounting. Backing is still physically contiguous and can
fail from fragmentation. CPU-local caches and virtually contiguous backing
remain later work. See [ADR 0032](decisions/0032-packed-kernel-heap.md).

Kernel entry stacks use guarded cells in a shared supervisor branch. Additional leaf tables extend the branch up to one GiB without replacing the parent links already cloned into user roots. Table owners remain retained through global TLB retirement. Legacy private user stacks and TLS occupy a reserved user VA arena; native runtime-owned threads do not consume those slots. These address budgets, physical memory, and retained resources still bound execution capacity. Frame metadata has no fixed four-GiB or 128-GiB size limit; raw boot-map and normalized-range counts remain bounded.

HHDM split-table ownership grows as finer alias permissions are needed. Published split tables and their owner-vector heap backing remain kernel-owned. Guest reclamation checks account for both retained resources separately from process allocations; table coalescing is not implemented.

Each process also owns explicit private virtual reservations. Reserve charges
virtual pages without allocating accessible data backing; commit supplies
zeroed RW/NX RAM from individually owned frames. Decommit retains the range,
and release retires the entire reservation. Account credit follows actual
retirement. Failed rollback retains owners and quarantines the process rather
than resuming a partial commit. These BSP-serialized mechanisms are independent
of the kernel heap and the eight-page shared-object interface.
Mapping records remain sorted for binary lookup and bulk removal. Uncommitted
reservations use interval metadata rather than one record per reserved page.
Intermediate page tables remain until process-space close; data release does
not implement table pruning.
[ADR 0026](decisions/0026-private-vm-and-atomic-waits.md) defines the contract;
user allocators and demand paging remain separate work.

Process-private word waits use aligned 32-bit words in owned RW mappings,
including ELF data and managed stack/TLS. Core validates and snapshots owned
RAM without borrowing user storage. Keys include space and mapping lifetimes
plus the exact word address. The image serializes comparison and queue admission
with wake after all user CPU leases end. The persistent wait table retains wait
charges and completes one result; image settlement writes saved return values
before scheduling. Decommit, unmap and execution-resource cleanup invalidate
affected queues before backing retirement. Wake is bounded and has no buffered
event. Shared waits and scalable parallel kernel policy remain separate work.

## User-space services

| Component | Responsibility |
| --- | --- |
| `init` | Start the initial services and distribute initial capabilities |
| `supervisor` | Monitor services, select restart policy, and coordinate recovery |
| `device-manager` | Discover devices, select drivers, and distribute device authority |
| `vfs` | Manage file namespaces, mounts, and filesystem routing |
| `pager` | Supply file-backed pages and manage caching policy |
| Drivers | Operate devices through restricted IRQ, MMIO, and DMA authority |
| Filesystems and networking | Implement storage and network protocols outside the kernel |

Initial programs and startup authority come from the boot image, without a VFS or storage dependency. Normal boot now starts a native supervisor with an immutable version-1 startup record. Its root inventory contains image authority, debug delegation, separate channel factories and assignable service resource accounts. User-space manifests select required roles, object kinds and maximum rights. SpawnManifest revalidates source authority and installs all child grants before execution; descriptive service labels grant no authority.

The configured supervisor registers an echo service through a private instance gate and gives each client a private directory gateway. Discovery uses the principal bound to that gateway, checks an allowlist and returns SEND alone. A separate bootstrap checkpoint capability coordinates validation; lookup authority cannot stop a service. Replacement consumes a fresh generation, withdraws the old entry and publishes a fresh endpoint. [ADR 0015](decisions/0015-service-startup-and-authority.md) records these boundaries. Dedicated device management, dynamic manifests, scalable discovery waits and supervisor recovery remain pending.

The earlier init/worker programs remain boot-test regressions. They exercise legacy two-grant startup, successive shared-process threads, independent TLS/FP state, retained exit observation and user-space replacement. Normal boot uses the new supervisor entry convention.

A separate bootstrap service probe receives a factory, peer image and diagnostic capability. Its peers create their own endpoints and advertise reduced SEND authority through IPC. The probe exercises request/reply, shared account exhaustion, exit and reconnection to a fresh endpoint, then retires before the initial supervisor starts. It retains the earlier two-grant convention as a regression alongside manifest startup.

A boot-minted device grant now connects restricted 32-bit MMIO operations to a separate Ring 3 HPET-comparator probe. IRQ entry records a coalesced event; BSP deferred work completes an owned notification. Device authority is revoked on process or IRQ-consumer death, and failed cleanup retains resources. [ADR 0011](decisions/0011-device-authority.md) defines this first device path. Direct MMIO maps, general device management, PCI routing, DMA pins, and IOMMU isolation remain pending.

## Compatibility and storage

The native ABI is independent of Linux and NT. ICE-Linux and ICE-NT each translate their own ABI into native mechanisms and services. Neither compatibility layer depends on the other. macOS/XNU compatibility is deferred.

BetAFS is an independent storage project. `userspace/filesystems/betafs/` is reserved for its Kryo adapter; the storage engine is not duplicated here.

The bootstrap retains read-only conventional PCI snapshots and constrained DMA
frame owners. An opt-in Ring 3 VirtIO block driver now submits actual read/write/
flush commands through private kernel queues. Only validated memory-object
offsets cross this boundary; raw PCI configuration, physical addresses and
hardware descriptors remain privileged. [ADR 0018](decisions/0018-mediated-virtio-block-transport.md)
defines the mediated transport and reset-before-release contract. Completion is
currently polled, with storage IRQ completion still pending.

The [BetAFS adapter contract](../userspace/filesystems/betafs/README.md) preserves
4096-byte blocks and durable flush. BetAFS keeps its normal Ring 3 std dependency.
Cryolite needs a native platform contract, libc, C++ runtime/library support and
Rust target/std platform support as separate deliverables.
[ADR 0019](decisions/0019-native-runtime-and-standard-libraries.md) defines their
implementation order; none of those runtime ports is complete.

The initial block service now shares the normal runtime with the supervisor.
It receives process-bound device authority and distinct read/write/control gates
before its suspended thread becomes Runnable. Separate clients use versioned
fixed-block IPC and memory loans. Normal boot retains the service; boot tests
stop it through explicit control authority and verify resource retirement.
This signed-disk configuration is not general device discovery or replacement.
[ADR 0020](decisions/0020-initial-block-service.md) defines the IPC and loan rules.

Native binary format is ELF64 AMD64; [ADR 0021](decisions/0021-native-elf-abi.md)
records the currently limited bootstrap ABI and the general native user ABI gate.

## Isolation and recovery

Mappings retain their backing memory. Closing a handle or terminating a service does not reclaim memory still referenced by another mapping, request, or device.

Service instances carry generations so a replacement cannot accept stale completions. Driver recovery also requires device quiescence and DMA revocation. Restarting a process alone is insufficient to stop a device.

Kernel memory access uses small, documented unsafe boundaries. Safe wrappers must retain the ownership or borrow that makes their mappings valid.

## Open decisions

The bootstrap memory model is specified in [ADR 0002](decisions/0002-user-address-spaces.md). [ADR 0003](decisions/0003-preemption-and-capabilities.md) adds task execution and authority, [ADR 0004](decisions/0004-waits-ipc-and-idle.md) waits and IPC, [ADR 0005](decisions/0005-shared-memory-and-delegation.md) shared backing, [ADR 0006](decisions/0006-native-elf-and-extended-state.md) native ELF execution and CPU state, and [ADR 0007](decisions/0007-monotonic-deadlines.md) monotonic deadlines. [ADR 0008](decisions/0008-process-startup-and-supervision.md) adds process startup and supervision. A shared image runtime now connects these mechanisms during normal boot and guest checks.

Experimental native calls use version 0 of the int 0x80 register interface. Their numbers and layout are not stable. Published IPC layout, user-space target and runtime, service quotas, heap efficiency, scheduling priorities, per-CPU run queues, and optional FRED support remain unresolved. The current scheduler uses round-robin selection with linear registry scans; kernel policy remains serialized on the BSP.

Every participating CPU must satisfy the x86-64-v3 hardware baseline before
user execution. The kernel keeps the conservative soft-float
`x86_64-unknown-none` target. An isolated v3 hard-float user target runs the
optional mixed C/C++/Rust probe with an owned SysV-style stack and separate
startup capability; the general native runtime remains pending. Intel is the
first Vulkan hardware target, with the native GPU-service
interface still open. [ADR 0022](decisions/0022-platform-baseline-and-performance.md)
records the requirements, performance work and decisions requiring review.
[ADR 0024](decisions/0024-native-bootstrap-stack.md) defines the bootstrap gate.
[ADR 0025](decisions/0025-static-tls-and-elf-metadata.md) extends it with truthful
mapped ELF metadata and runtime-owned compiler static TLS. The full mlibc TCB
and user allocator remain pending. Explicit private VM and atomic private waits
provide page-backing and synchronization boundaries through
[ADR 0026](decisions/0026-private-vm-and-atomic-waits.md).
[ADR 0027](decisions/0027-runtime-owned-thread-memory.md) adds runtime-owned
thread memory and completion ordering. Full POSIX attributes, floating-point
environment and signal inheritance, cleanup handlers and destructors remain
runtime work; the raw kernel mechanism does not implement pthreads.
SOF means Substitute-On-the-Fly across Cryolite. Kryo's security and SOF managers
own only kernel admission and replacement; a separate Ring 3 manager owns
service, driver and filesystem SOF. Kernel capability, VM, IPC and DMA lifetime
enforcement still applies to those services. Kernel replacement requires two
full admission checks and a versioned live-state handoff; it is not implemented.
See the [security policy](../SECURITY.md) and
[kernel SOF contract](decisions/0030-kernel-sof.md).
