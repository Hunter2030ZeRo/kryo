# Roadmap

Develop one verifiable capability at a time. The [design contract](design-contract.md) defines the required behavior; [status](status.md) records the completed work.

| Stage | Completion criterion | Status |
| --- | --- | --- |
| P01: Boot memory | Discover boot resources, distinguish address types, and preserve reserved memory | Complete |
| P02: Physical frames | Normalize usable ranges, reserve runtime-sized frame metadata, establish ownership, and test allocation, return, exhaustion, and failed initialization | Runtime metadata and high-address allocation implemented; current large-memory evidence is recorded in status |
| P03: Virtual memory | Own address spaces and mappings; enforce permissions and alias policy; reclaim frames after TLB completion; test protection and guard pages | Growing VM metadata, explicit private reserve/commit/decommit/release, shared backing, guarded entry stacks and acknowledged SMP retirement implemented; demand paging and table pruning pending |
| P04: User entry | CPU-local state, user/kernel stacks, validated IRETQ entry and return, native calls, and user fault isolation | Static native ELF, CPU-local SMP FP/SIMD contexts, runtime-owned stack/TLS leases, FS setup, and fault isolation implemented; general runtime and stable ABI pending |
| P05: Tasks and IPC | Timer, context switching, wait/wakeup, kernel objects, capabilities, endpoints, notifications, and shared memory | SMP execution, growing metadata, deadlines, delegation, native channels, atomic private word waits and shared resource accounts implemented; shared waits, CPU-time budgets and scheduling scalability pending |
| P06: Services and DMA | Boot image, init, supervisor, device authority, DMA lifetime, and service failure recovery | Native supervisor, versioned atomic startup grants, role allowlists, private discovery gateways, generations and service replacement implemented for a configured inventory; mediated HPET IRQ, read-only PCI discovery and owned DMA storage implemented; private queued hardware DMA and Ring 3 block commands validated; general assignment and broader recovery pending |

## Next: service authority and runtime

The native platform minimum is x86-64-v3; Intel is the first Vulkan hardware
target. An opt-in mixed-language probe now tests the v3 hard-float convention
and owned initial stack. The
[static TLS extension](decisions/0025-static-tls-and-elf-metadata.md) adds mapped
ELF metadata and runtime-owned compiler TLS. Explicit private VM now supplies
page backing with separate virtual/RAM charges and retirement. Atomic private
32-bit waits add compare-before-park, deadlines, bounded wake and invalidation
before mapping retirement. Native thread creation now borrows runtime-prepared
stack/TLS ranges, protects guards and releases their execution lease before
join publication. Full mlibc startup, a user allocator and pthread cleanup
remain pending. Use the [thread-memory contract](decisions/0027-runtime-owned-thread-memory.md)
and the
[memory/wait contracts](decisions/0026-private-vm-and-atomic-waits.md)
when connecting mlibc synchronization. Review
libc selection, parallel kernel ownership and the GPU-service interface before
freezing their ABI. Measure IPC, scheduler, heap and I/O costs alongside runtime
porting rather than postponing them until desktop integration. See
[ADR 0022](decisions/0022-platform-baseline-and-performance.md).

CPU-local state, Limine AP startup, parallel user execution, remote wakeups, and acknowledged TLB retirement now connect through the shared runtime. [ADR 0010](decisions/0010-smp-execution.md) describes execution ownership; [ADR 0012](decisions/0012-growing-registries.md) extends CPU sets to 255 directed xAPIC processors and adds fallible metadata growth. Kernel policy remains serialized on the BSP. Per-CPU run queues, parallel syscall processing, hotplug, topology policy, and physical performance validation remain separate work.

Frame metadata now scales with the normalized Usable RAM inventory. The image reserves its backing before initializing the frame pool or heap. [ADR 0013](decisions/0013-runtime-frame-metadata.md) records bootstrap exclusion, physical-address selection, and the large-memory proof scope. Further memory work includes fragmented metadata backing, memory hotplug, NUMA policy, and physical-hardware validation.

Normal boot starts a native supervisor with explicit root authority. Versioned manifests install role-labeled grants before child execution. Private instance registration and client discovery gateways bind service policy to capabilities; claimed IDs do not authenticate it. Shared resource accounts now bound handles, waits, live threads and shared payload pages, with additional per-process VM limits. Ordinary children inherit accounts and restart retains usage. [ADR 0015](decisions/0015-service-startup-and-authority.md) defines startup/discovery and [ADR 0016](decisions/0016-service-resource-budgets.md) defines resource lifetime. Validated private DMA queues now carry Ring 3 block commands. A configured persistent block IPC service now connects clients through private read/write gates. The next path is the dedicated user ABI, native runtime, VFS/BetAFS and storage IRQ completion. CPU elapsed-time accounting, scalable service events and dynamic configuration remain separate work. The current HPET grant validates restricted registers and IRQ notification; it is not a general device manager.

A general supported runtime/sysroot, full TLS/TCB, libc/std and dynamic linking remain necessary for ordinary applications. The freestanding target, static TLS and explicit private-memory API do not establish those ports. Shared pages and copied capabilities are implemented; buffer loans, bulk queues, object revocation, DMA pins, and IOMMU isolation remain pending.

## Service and application checkpoints

1. Start init from the boot image, distribute initial authority, then start supervisor and device management. Test service termination, restart generations, and stale requests.
2. Connect real IRQ and device authority, PCI/VirtIO or other initial drivers, DMA isolation, VFS, storage, and networking. A user-space driver must not retain DMA access after its buffers are reclaimed.
3. Connect ICE to real process/address-space execution and add the required runtime, files, PTY, input, and timing behavior for one terminal emulator. Host ELF parsing alone does not satisfy this checkpoint.
4. Run Velocity against actual storage and its required package operations. Add the graphics, input, process, and compatibility services required by KDE, then verify a real desktop session.

Extend SMP scheduling after the corresponding ownership and failure-path checks pass. QEMU functional checks do not establish hardware performance, energy use, or device recovery.

## PCI and storage foundation

Conventional PCI discovery and owned DMA buffers now connect to actual Ring 3
VirtIO block read/write/flush through private validated queues. The opt-in
validation path uses polling and a disposable signed disk. Configured initial block IPC now works; storage IRQ completion, external
publication and general assignment remain pending. See
[ADR 0018](decisions/0018-mediated-virtio-block-transport.md).

BetAFS keeps its normal Ring 3 std dependency. Its adapter must preserve 4096-byte
blocks and actual durable flush. The Cryolite Rust target/std platform support
will share native services with libc and the C++ runtime; their implementation
order is defined in [ADR 0019](decisions/0019-native-runtime-and-standard-libraries.md).

## Integration

Connect ICE to actual address spaces and execution. Add the BetAFS block/VFS adapter, then the runtime, PTY, terminal, and Velocity paths. KDE follows the required process, graphics, input, and runtime services.

ICE and BetAFS retain independent host test suites. Their host results do not establish Kryo integration.

## Later work

Add per-CPU queues, efficient heap backing and slabs, memory hotplug and NUMA policy, x2APIC, large pages, CPU-local caches, and physical hardware support after the corresponding correctness tests pass. The initial heap already owns contiguous frame runs, but fragmentation and whole-page allocation costs remain. Preserve execution ownership and acknowledged memory retirement across scheduling changes.

Keep kernel FP/SIMD disabled by default. General user runtimes require initialization and preservation of all CPU state used by their ABI.
