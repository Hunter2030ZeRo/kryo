# Repository layout

The workspace contains four packages. Reserved directories use `.gitkeep`; add Rust modules and packages when their implementation exists.

## Packages and boot inputs

| Path | Responsibility |
| --- | --- |
| `boot/` | Linker script, Limine configuration, and initial-image inputs |
| `hal/` | Architecture-specific mechanisms and hardware access |
| `interfaces/` | Native ABI and service protocol representations |
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
| `hal/src/arch/x86_64/gdt.rs` | GDT, CS/SS, and TR loading |
| `hal/src/arch/x86_64/tss.rs` | BSP TSS and double-fault stack |
| `hal/src/arch/x86_64/interrupts/idt.rs` | Exception gates and handlers |
| `hal/src/arch/x86_64/paging/` | HHDM access and BSP scratch page tables |
| `hal/src/debug/console.rs` | Early formatted and emergency output |
| `hal/src/io/framebuffer.rs` | Checked 32bpp framebuffer writes |
| `kernel/core/src/mm/frame/` | Ranges, normalization, frame ownership, and allocation |
| `kernel/core/tests/` | Host tests for memory mechanisms |
| `kernel/image/src/main.rs` | Entry, panic handling, framebuffer exercise, and breakpoint return |
| `kernel/image/src/boot.rs` | Limine requests and boot module wiring |
| `kernel/image/src/boot/memory.rs` | Boot memory interpretation and frame-pool activation |
| `kernel/image/src/boot/framebuffer.rs` | Limine framebuffer conversion |
| `kernel/image/src/boot/checks.rs` | Guest memory tests, gated by boot-tests |
| `kernel/image/build.rs` | Linker arguments for the kernel binary |
| `tools/boot_smoke.py` | Bounded headless QEMU regression runner |

## Reserved kernel work

HAL's x86-64 `boot/`, `apic/`, `context/`, `syscall/`, `smp/`, `iommu/`, and `asm/` directories reserve CPU entry, interrupt controllers, context switching, syscall entry/return, AP startup, DMA isolation, and assembly support. The existing `cpu.rs`, `interrupts/traps.rs`, and `interrupts/fred.rs` files are placeholders.

Core's reserved memory modules are:

- `mm/vm/`: address spaces, mappings, fault handling, and pager integration.
- `mm/heap/`: kernel heap allocation; allocator selection remains open.
- `mm/shared/`: shared memory objects, buffers, and loans.
- `mm/dma/`: pinning, DMA mappings, and revocation.

Other reserved core modules are `init/`, `task/`, `sched/`, `object/`, `capability/`, `ipc/`, `interrupt/`, `syscall/`, `sync/`, `time/`, `security/`, and `diagnostics/`.

Objects provide identity and lifetime; capabilities provide authority; security provides common validation and audit rules. Keep one implementation of handle lookup and rights checking. VFS and NT-specific object policy remain outside core.

## Interfaces

The reserved `interfaces/src/` modules are `syscall/`, `handle/`, `ipc/`, `memory/`, `device/`, `filesystem/`, and `service/`. They define public representations and protocols when those interfaces are implemented; object tables and policy belong elsewhere.

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

All are reserved. No user-space executable is built yet.

## Compatibility and tests

ICE's `linux/` and `nt/` paths independently adapt their ABIs to native facilities. `compat/ice/userspace/` reserves loader and runtime integration.

The root test directories reserve boot, syscall, IPC, recovery, compatibility, performance, and unit scenarios. Current host tests live in core and HAL; guest tests are driven by the smoke tool. See [tests](../tests/README.md).

`tools/xtask/` and `tools/debug/` remain reserved. Current build and run commands are defined in the Makefile.
