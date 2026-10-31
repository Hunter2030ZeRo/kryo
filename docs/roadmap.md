# Roadmap

Develop one verifiable capability at a time. The [design contract](design-contract.md) defines the required behavior; [status](status.md) records the completed work.

| Stage | Completion criterion | Status |
| --- | --- | --- |
| P01: Boot memory | Discover boot resources, distinguish address types, and preserve reserved memory | Complete |
| P02: Physical frames | Normalize usable ranges, subtract reservations, establish ownership, and test allocation, return, exhaustion, and failed initialization | Complete |
| P03: Virtual memory | Own address spaces and mappings; enforce permissions and alias policy; reclaim frames after TLB completion; test protection and guard pages | Boot mapping primitive complete; general VM pending |
| P04: User entry | CPU-local state, user/kernel stacks, validated IRETQ entry and return, native calls, and user fault isolation | Pending |
| P05: Tasks and IPC | Timer, context switching, wait/wakeup, kernel objects, capabilities, endpoints, notifications, and shared memory | Pending |
| P06: Services and DMA | Boot image, init, supervisor, device authority, DMA lifetime, and service failure recovery | Pending |

## Next: Address spaces

Build mapping ownership on the existing frame pool and HAL mechanisms. Separate user and kernel mappings. Define executable-memory permissions across all aliases, including the HHDM. Verify read-only/NX protection, guard pages, failed mappings, and reclamation before entering Ring 3.

## Integration

Connect ICE to actual address spaces and execution. Add the BetAFS block/VFS adapter, then the runtime, PTY, terminal, and Velocity paths. KDE follows the required process, graphics, input, and runtime services.

ICE and BetAFS retain independent host test suites. Their host results do not establish Kryo integration.

## Later work

Add SMP, contiguous allocations, large pages, CPU-local caches, and physical hardware support after the corresponding correctness tests pass. CPU-local TSS, stacks, execution ownership, and memory lifetime must be accounted for before enabling SMP.

Keep kernel FP/SIMD disabled by default. General user runtimes require initialization and preservation of all CPU state used by their ABI.
