# ADR 0002: User address spaces and bootstrap entry

Date: 2026-10-31

Status: Implemented for BSP bootstrap; scheduling and service authority remain separate work.

The execution limits below describe this initial stage. [ADR 0003](0003-preemption-and-capabilities.md) extends it with resumable contexts, timer preemption, guarded entry stacks, and task-control capabilities.

## Context

Cryolite places drivers, filesystems, networking, and other services in user space. Each service needs private memory, explicit resource ownership, and a fault path that leaves the kernel available.

The initial implementation must establish those memory boundaries before a public syscall ABI or shared-memory service protocol is introduced.

## Address-space ownership

Core owns the root, intermediate tables, and mapped data through frame-allocation tokens. The HAL backend edits page tables but does not own or release frames. A small unsafe backend contract connects the two; core tests use a model backend.

Each address space has private lower-half tables and inherited supervisor-only upper mappings. Mapping metadata uses explicit capacities; exhaustion is an error. Initial mappings own individual 4 KiB frames. Shared memory objects and pin references are future extensions.

An exclusive execution borrow prevents mutation and close while a space is running. Mutation is permitted only while the root is inactive. Closing removes data mappings, retires translation access, restores aliases, and then releases data, table, and root tokens. Failed cleanup retains ownership for retry. Dropping an unclosed space leaks its allocation rather than freeing active memory.

## Permissions and aliases

User pages support RO, RW, and RX. Reject writable-executable mappings, null pages, kernel addresses, noncanonical addresses, and invalid alignment.

Before cloning user roots, mark the managed HHDM range supervisor-only and NX, enable CR0.WP, and reject unexpected boot aliases. The kernel image mapping remains executable in its own slot and does not map Usable allocation frames.

Initialize code privately, then remove HHDM write permission before publishing its user RX mapping. Read-only user data also has a read-only HHDM alias. Restore a writable NX alias only after the user mapping is retired.

Split huge HHDM mappings when a frame needs 4 KiB permissions. Validate all frames and prepare replacement tables before publication. These shared kernel tables retain separate ownership for the kernel lifetime. Per-space teardown must not release them. Unsupported large-page PAT encodings are rejected.

Global TLB entries are invalidated when kernel aliases change. User mappings are non-global; PCID is disabled and returning to the boot CR3 retires their translations. SMP requires shootdown acknowledgments before reclamation.

## Entry and faults

The HAL enters Ring 3 through IRETQ with validated addresses and fixed selectors/flags. Initial registers and FS/GS state are cleared. A separate retained TSS RSP0 stack handles privilege entry; double faults use their own IST.

The trampoline saves and restores kernel registers, stack, CR3, CPU control state, segment selectors, and FS/GS bases. Trap stubs normalize hardware error frames and clear DF before calling Rust.

A DPL3 int 0x80 gate ends the bootstrap execution and returns a status from RDI. This is an experimental exit mechanism, not a published native or Linux syscall ABI. Unconfigured SYSCALL/SYSENTER entry is disabled. FP/SIMD is trapped until per-thread state preservation exists.

User faults return a recorded outcome to the kernel caller. Kernel faults remain fatal. The kernel never dereferences a user pointer in this exit/fault path.

## Scope

Execution is synchronous on the BSP with IRQs disabled, four-level paging, and no PCID. Test programs are bounded scalar instruction sequences. There is no timer, preemption, scheduler, or protection against an infinite user loop yet.

Ring 3 does not grant file, device, or IPC authority. Those operations require future capability-checked interfaces. DMA isolation also requires an IOMMU and a device-lifetime protocol.

The boot stack is explicitly requested at 256 KiB to accommodate unoptimized initialization. Per-thread stacks, guarded kernel entry stacks, shared mappings, quotas, and general runtime state are pending.

See [status](../status.md) for verification results and remaining work.
