# Roadmap

Develop one verifiable capability at a time. The [design contract](design-contract.md) defines the required behavior; [status](status.md) records the completed work.

| Stage | Completion criterion | Status |
| --- | --- | --- |
| P01: Boot memory | Discover boot resources, distinguish address types, and preserve reserved memory | Complete |
| P02: Physical frames | Normalize usable ranges, reserve runtime-sized frame metadata, establish ownership, and test allocation, return, exhaustion, and failed initialization | Runtime metadata and high-address allocation implemented; current large-memory evidence is recorded in status |
| P03: Virtual memory | Own address spaces and mappings; enforce permissions and alias policy; reclaim frames after TLB completion; test protection and guard pages | Growing VM metadata, shared backing, guarded entry stacks, and acknowledged SMP retirement implemented; general paging policy pending |
| P04: User entry | CPU-local state, user/kernel stacks, validated IRETQ entry and return, native calls, and user fault isolation | Static native ELF, CPU-local SMP FP/SIMD contexts, FS setup, and fault isolation implemented; general runtime and stable ABI pending |
| P05: Tasks and IPC | Timer, context switching, wait/wakeup, kernel objects, capabilities, endpoints, notifications, and shared memory | SMP execution, growing metadata, deadlines, shared memory, copy delegation, native channel creation and shared channel accounting implemented; general quotas and scheduling scalability pending |
| P06: Services and DMA | Boot image, init, supervisor, device authority, DMA lifetime, and service failure recovery | Native supervisor, versioned atomic startup grants, role allowlists, private discovery gateways, generations and service replacement implemented for a configured inventory; mediated HPET authority/IRQ path implemented; general devices, DMA and broader recovery pending |

## Next: service authority and runtime

CPU-local state, Limine AP startup, parallel user execution, remote wakeups, and acknowledged TLB retirement now connect through the shared runtime. [ADR 0010](decisions/0010-smp-execution.md) describes execution ownership; [ADR 0012](decisions/0012-growing-registries.md) extends CPU sets to 255 directed xAPIC processors and adds fallible metadata growth. Kernel policy remains serialized on the BSP. Per-CPU run queues, parallel syscall processing, hotplug, topology policy, and physical performance validation remain separate work.

Frame metadata now scales with the normalized Usable RAM inventory. The image reserves its backing before initializing the frame pool or heap. [ADR 0013](decisions/0013-runtime-frame-metadata.md) records bootstrap exclusion, physical-address selection, and the large-memory proof scope. Further memory work includes fragmented metadata backing, memory hotplug, NUMA policy, and physical-hardware validation.

Normal boot starts a native supervisor with explicit root authority. Versioned manifests install role-labeled grants before child execution. Private instance registration and client discovery gateways bind service policy to capabilities; claimed IDs do not authenticate it. A configured inventory exercises reduced SEND grants, independent channel accounts and service replacement. [ADR 0014](decisions/0014-native-channel-creation.md) defines channel lifetime and [ADR 0015](decisions/0015-service-startup-and-authority.md) defines startup and discovery. Next add general resource budgets, scalable service events, dynamic startup configuration and device authority. The HPET bootstrap grant validates restricted register access and IRQ notification; general driver ports still need PCI/IRQ discovery, MMIO mapping lifetimes, DMA pins and IOMMU retirement.

A general user target/runtime, ELF TLS templates, libc/std, dynamic linking, and libsys startup conventions remain necessary for ordinary applications. Shared pages and copied capabilities are implemented; buffer loans, bulk queues, object revocation, DMA pins, and IOMMU isolation remain pending.

## Service and application checkpoints

1. Start init from the boot image, distribute initial authority, then start supervisor and device management. Test service termination, restart generations, and stale requests.
2. Connect real IRQ and device authority, PCI/VirtIO or other initial drivers, DMA isolation, VFS, storage, and networking. A user-space driver must not retain DMA access after its buffers are reclaimed.
3. Connect ICE to real process/address-space execution and add the required runtime, files, PTY, input, and timing behavior for one terminal emulator. Host ELF parsing alone does not satisfy this checkpoint.
4. Run Velocity against actual storage and its required package operations. Add the graphics, input, process, and compatibility services required by KDE, then verify a real desktop session.

Extend SMP scheduling after the corresponding ownership and failure-path checks pass. QEMU functional checks do not establish hardware performance, energy use, or device recovery.

## Integration

Connect ICE to actual address spaces and execution. Add the BetAFS block/VFS adapter, then the runtime, PTY, terminal, and Velocity paths. KDE follows the required process, graphics, input, and runtime services.

ICE and BetAFS retain independent host test suites. Their host results do not establish Kryo integration.

## Later work

Add per-CPU queues, efficient heap backing and slabs, memory hotplug and NUMA policy, x2APIC, large pages, CPU-local caches, and physical hardware support after the corresponding correctness tests pass. The initial heap already owns contiguous frame runs, but fragmentation and whole-page allocation costs remain. Preserve execution ownership and acknowledged memory retirement across scheduling changes.

Keep kernel FP/SIMD disabled by default. General user runtimes require initialization and preservation of all CPU state used by their ABI.
