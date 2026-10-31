# Architecture

Kryo is the microkernel of Cryolite. It uses Rust `no_std`, targets x86-64, and boots through Limine.

This document defines subsystem boundaries. The [design contract](design-contract.md) specifies ownership and synchronization rules. [Status](status.md) records implemented behavior.

## Kernel boundary

Ring 0 provides CPU control, memory management, execution, capabilities, and communication mechanisms. General device drivers, VFS, filesystems, and networking run as separate Ring 3 services.

Interrupt controllers, the MMU, and early debug output remain kernel mechanisms. Service discovery, driver selection, filesystem routing, and restart policy belong in user space.

The design targets low overhead and fault isolation. Small requests use kernel IPC; bulk transfers use registered shared memory. Performance decisions require measurements of copying, scheduling, and wakeup costs.

## Packages

| Package | Responsibility |
| --- | --- |
| `hal` | CPU entry, registers, exception gates, page tables, TLB operations, and hardware access |
| `kernel/core` | Resource ownership, address spaces, tasks, kernel objects, capabilities, IPC, and scheduling |
| `kernel/image` | Limine responses, component initialization, kernel entry, and panic handling |
| `interfaces` | Versioned user-visible representations and protocols |

Current internal Cargo dependencies:

```text
kernel/image --> kernel/core
             --> hal

interfaces       no consumers yet
```

Core currently uses only `spin`; it has no HAL or interfaces dependency. Hardware operations are connected by the image package. Future core-to-HAL interfaces must preserve this separation and must not introduce a HAL-to-core dependency cycle. Prefer static dispatch where it keeps the interface simple.

User programs call the native ABI through their runtime or `libsys`. They do not link privileged core or HAL code.

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

Initial programs and service manifests are supplied by a boot image. Starting `init` must not depend on an already running VFS or storage driver.

## Compatibility and storage

The native ABI is independent of Linux and NT. ICE-Linux and ICE-NT each translate their own ABI into native mechanisms and services. Neither compatibility layer depends on the other. macOS/XNU compatibility is deferred.

BetAFS is an independent storage project. `userspace/filesystems/betafs/` is reserved for its Kryo adapter; the storage engine is not duplicated here.

## Isolation and recovery

Mappings retain their backing memory. Closing a handle or terminating a service does not reclaim memory still referenced by another mapping, request, or device.

Service instances carry generations so a replacement cannot accept stale completions. Driver recovery also requires device quiescence and DMA revocation. Restarting a process alone is insufficient to stop a device.

Kernel memory access uses small, documented unsafe boundaries. Safe wrappers must retain the ownership or borrow that makes their mappings valid.

## Open decisions

The following remain unresolved: native syscall numbers and register layout, published IPC layout, user-space target and runtime, heap and contiguous-frame allocation, scheduling policy, SMP details, and optional FRED support.

The kernel currently uses the built-in `x86_64-unknown-none` target without an `x86-64-v3` requirement. Kernel hot replacement is a separate research topic from user-space service replacement.
