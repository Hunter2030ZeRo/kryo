# Repository layout regression: 2026-10-30

Historical snapshot from the layout migration. Current implementation status is maintained in [status](../status.md).

## Scope

The migration introduced HAL, interfaces, kernel core, and kernel image packages while retaining the existing boot path:

```text
_start -> init_gdt -> init_idt -> A -> int3 -> B -> C -> cli/hlt
```

At this snapshot, GDT/CS/SS and the breakpoint gate were connected. The TSS file contained a 16 KiB aligned stack declaration; TSS/TR/IST and double-fault handling were not connected. Core, interfaces, user-space services, and compatibility integration were unimplemented.

## Changes

| Original location | Destination | Change |
| --- | --- | --- |
| `src/main.rs` | `kernel/image/src/main.rs` | Import initialization from HAL; retain execution flow |
| `src/arch/` | `hal/src/arch/` | Expose init_gdt and init_idt |
| `src/debug/` | `hal/src/debug/` | Preserve contents and existing disconnected state |
| Linker and Limine files | `boot/` | Preserve contents |
| Root manifest | Workspace and package manifests | Preserve dependency versions and profiles |
| Linker rustflags | `kernel/image/build.rs` | Apply the linker script to the kernel binary only |
| Makefile | Same location | Update paths, package selection, and profile handling |

The candidate buddy and talc dependencies were retained but inactive at this date. Existing TSS work and build outputs were preserved.

## Results

Tests used separate build and ISO locations.

| Check | Result |
| --- | --- |
| Locked offline workspace check | Passed |
| Debug and release ISO builds | Passed |
| QEMU TCG boot, both profiles | Exact ABC |
| Segment registers | CS=0x0008, SS=0x0010 |
| Descriptor tables | GDT base matched the ELF symbol; GDT limit 0x17, IDT limit 0xfff |
| Final CPU state | HLT=1, IF=0 |
| Preservation | TSS work, linker/Limine contents, dependency checksums, and existing outputs retained |
| Structure | Relative links valid; four workspace packages |

The [raw validation record](2026-10-30-layout.json) contains register values, commands, and source hashes. Unused imports/dead code in the incomplete TSS and unused dependency warnings remained.

These results cover layout regression only. They do not establish TSS/IST behavior, user execution, IPC, drivers, or hardware performance.
