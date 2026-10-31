# Roadmap

Develop one verifiable capability at a time. The [design contract](design-contract.md) defines the required behavior; [status](status.md) records the completed work.

| Stage | Completion criterion | Status |
| --- | --- | --- |
| P01: Boot memory | Discover boot resources, distinguish address types, and preserve reserved memory | Complete |
| P02: Physical frames | Normalize usable ranges, subtract reservations, establish ownership, and test allocation, return, exhaustion, and failed initialization | Complete |
| P03: Virtual memory | Own address spaces and mappings; enforce permissions and alias policy; reclaim frames after TLB completion; test protection and guard pages | Bounded user VM and guarded task entry stacks implemented; sharing and SMP retirement pending |
| P04: User entry | CPU-local state, user/kernel stacks, validated IRETQ entry and return, native calls, and user fault isolation | BSP resumable scalar contexts and experimental calls implemented; general runtime state and stable ABI pending |
| P05: Tasks and IPC | Timer, context switching, wait/wakeup, kernel objects, capabilities, endpoints, notifications, and shared memory | BSP preemption, task/object/handle lifetime, waits, inline endpoint IPC, and notifications implemented; capability transfer and shared memory pending |
| P06: Services and DMA | Boot image, init, supervisor, device authority, DMA lifetime, and service failure recovery | Pending |

## Next: Memory sharing and delegation

Add shared-memory objects with mapping/reference lifetimes and quotas. Add bounded capability transfer with receiver capacity reserved before acceptance; preserve sender resources on failure before acceptance. Retain accepted resources through cancellation and peer termination. Verify host models before guest shared-buffer and transfer tests.

Keep the current BSP scheduler and scalar context scope explicit. Waits and inline IPC now have host and guest coverage. Timer counts require calibration before they can represent deadlines, timeouts, or a millisecond quantum. Add FP/SIMD and runtime state preservation before general Rust/C programs.

Extend VM with shared memory objects, reference/pin lifetimes, quotas, and SMP shootdown acknowledgments. Normal boot still needs initial-image loading and service startup.

## Integration

Connect ICE to actual address spaces and execution. Add the BetAFS block/VFS adapter, then the runtime, PTY, terminal, and Velocity paths. KDE follows the required process, graphics, input, and runtime services.

ICE and BetAFS retain independent host test suites. Their host results do not establish Kryo integration.

## Later work

Add SMP, contiguous allocations, large pages, CPU-local caches, and physical hardware support after the corresponding correctness tests pass. CPU-local TSS, stacks, execution ownership, and memory lifetime must be accounted for before enabling SMP.

Keep kernel FP/SIMD disabled by default. General user runtimes require initialization and preservation of all CPU state used by their ABI.
