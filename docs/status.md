# Implementation status

Snapshot: 2026-10-31. This file records implemented behavior. Requirements for future subsystems are in the [design contract](design-contract.md).

## Implemented

- Limine boot requests for the memory map, HHDM, executable addresses, framebuffer, and four-level paging.
- GDT and CS/SS setup; TSS/TR and a double-fault IST; IDT, breakpoint return, and GP/PF diagnostics.
- Checked physical ranges, page alignment, reservation merging/subtraction, and Usable RAM normalization.
- A dense bitmap frame pool with summary bits, allocation hints, one-time activation, and explicit return.
- Non-cloneable, lifetime-bound `OwnedFrame` tokens and wrong-pool return rejection.
- Checked HHDM address conversion and owned-frame zeroing.
- `BootPageSlot`: a BSP mapping in a dedicated, initially unused PML4 branch.
- Supervisor RW+NX leaf mapping, local TLB invalidation, branch removal, and table-frame reclamation.
- Early debug output and a 32bpp RGB framebuffer writer.

Core owns physical allocations. HAL performs memory and page-table access. Image interprets boot responses and connects initialization.

## Recorded verification

The [evidence manifest](validation/2026-10-31-memory/evidence.json) contains the environment and results.

| Check | Result |
| --- | --- |
| no_std workspace check | Passed |
| Rust format check | Passed |
| Clippy with all features and warnings denied | Passed |
| Host tests | 33 passed: ranges 5, normalization 11, pool 12, HAL paging 5 |
| Normalization model | 2,000 generated maps compared with an independent page oracle |
| Frame model | 20,000 operations, exhaustion, return, reservations, failed initialization, and host-thread exclusion |
| Ownership compile checks | Rejected cloning, private-field construction, double return, and a token outliving its pool |
| QEMU TCG, debug and release | One boot each; frame checks, full-page zeroing, 128 map/unmap cycles, and final ABC passed |
| Normal debug boot | Pool activation and ABC passed with boot tests disabled |

The mapping test retains three table frames and alternates two data frames. It writes all 512 u64 words through the scratch address and checks them through the HHDM alias. It also checks duplicate mapping, unmapping an empty slot, and construction in an occupied PML4 slot.

Eight branch creation/removal cycles perform 128 mappings in total. The free-frame count returns to its initial value, including table frames. The zeroing test fills an entire frame with nonzero data before checking that every byte was cleared.

Logs: [debug](validation/2026-10-31-memory/debug.log), [release](validation/2026-10-31-memory/release.log), and [normal debug](validation/2026-10-31-memory/normal-debug.log). The runner terminated its own QEMU after observing ABC; these results do not establish guest shutdown or hardware performance.

The earlier layout-only regression is recorded in the [2026-10-30 notes](validation/2026-10-30-layout-notes.md).

## Limits

Boot mappings require one CPU, disabled maskable IRQs, an unchanged CR3, and exclusive page-table access. PCID and five-level paging are rejected.

The static pool configuration supports 288 normalized ranges and up to 4 GiB of managed RAM in total. This is a metadata capacity limit, not a physical-address limit. Capacity overflow is an error.

Only the new scratch leaf is NX. Inherited HHDM permissions remain unchanged, including RWX aliases. Full backing-frame W^X is not implemented.

`BootPageSlot` is a boot primitive, not a general address space. Bootloader-reclaimable memory, ACPI memory, and active boot page tables remain reserved.

Kernel heap allocation, general VM, user entry, syscalls, scheduling, capabilities, IPC, and DMA support are pending. ICE execution and BetAFS I/O are not integrated.

## Next work

Implement address-space and mapping ownership, user/kernel separation, and executable-memory permissions across aliases. Verify protection and reclamation before adding Ring 3 entry and user fault isolation.
