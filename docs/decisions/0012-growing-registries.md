# Growing registries and CPU sets

Date: 2026-11-01

## Decision

Normal service execution uses fallible, growing metadata for processes, threads,
process membership, objects, handles, waits, observations, and VM mappings and
table owners. The previous two-process and four-thread storage was bootstrap
fixture capacity. It must not constrain normal service execution.

Core `Slots` supports explicit inline storage and growing `alloc::Vec` storage.
Inline constructors remain useful for deterministic exhaustion tests. Growing
constructors allocate only when another slot is needed. Optional explicit limits
support policy and failure models; production service quotas remain pending.

CPU sets span multiple 64-bit words. The configured registry holds 255 logical
processors with directed xAPIC identities 0..254. APIC 255 remains the broadcast
destination and is rejected as an individual target. This removes the one-word
mask restriction; it does not add x2APIC, hotplug, or topology policy.

## Ownership and publication

Slot indices and generations remain unchanged when backing storage moves.
Vacant generational slots remain allocated for the registry's lifetime. A slot
retires before generation wrap; growth never resets its history. Strong object
references retain a table and identity rather than an address into its vector.
The arena lock covers lookup and reference acquisition.

All user jobs return their owned CPU state and acknowledge the boot CR3 before
the BSP mutates registries. APs retain no Rust references into moving metadata.
CPU-local GDT/TSS, stacks, and entry state retain their stable storage.

Creation reserves metadata, initial process membership, and result-handle capacity
before runnable publication. Allocation failure returns the unpublished owner.
Failed VM or stack cleanup remains retained for retry. Wait registration reserves
transfer, sleep, and observer side records before blocking; completion delivery
does not need to grow those records. Accepted capability grants still commit
before sender cleanup or receiver wakeup.

Process and thread reaping return execution resources after explicit retirement.
Their registry slot history remains available for stale-ID checks. VM close can
release its mapping metadata after mappings, aliases, table access, and the root
have retired. Dropping an unclosed owner retains its unsafe-to-reclaim resources.

## Initial allocator

The image supplies a page allocator backed by contiguous physical runs. The frame
pool serializes run allocation with ordinary single-frame allocation. Runs cannot
cross physical holes. Each heap allocation retains its `OwnedRun` in an aligned
private prefix; deallocation moves that token out before returning its frames.
The retained HHDM provides supervisor writable aliases, with NX established before
user execution. Allocator paths exclude local IRQ reentry and acquire no VM or
object locks.

Metadata paths use fallible reservation. A failed reallocation keeps the old
buffer and its owners. Neither allocation failure nor quota exhaustion should
panic during an ordinary user operation. Boot invariants may halt with diagnostics.

The allocator uses whole pages for each allocation. It can waste space on small
vectors and can fail from physical fragmentation despite sufficient total free
RAM. Slabs, virtual heap backing, and CPU-local caches require separate work.

## Stack mappings and CPU snapshots

Kernel entry stacks occupy fixed guarded cells in a shared supervisor branch.
Additional leaf tables extend one already-linked directory, so existing user
roots see the same parent branch. Software records and frame owners are retained
before parent publication. A failed global shootdown leaves the added table
linked and pending; retry must finish before mapping more pages or dispatching
users. Detach retires every shared root and translation before returning tables.

HHDM split-table ownership also uses growing metadata. Published split tables
remain kernel-owned even after the user mapping that needed finer permissions
retires. Final frame accounting includes both these permanent tables and the
actual heap pages backing their retained owner vector. Successful task cleanup
must restore the remaining allocations exactly; permanent ownership is not an
allowance for an unexplained difference.

The branch has a one-GiB address budget, roughly 52,000 guarded 16-KiB stack cells.
This is a virtual-address ceiling, not a supported thread-count guarantee. The
frame pool, retained alias tables, save areas, and other allocations can exhaust
first. Private user stacks and TLS use a reserved user-address arena.

Atomic CPU-set snapshots read each word independently. They are not a linearizable
snapshot of concurrent membership changes. Startup, execution leases, shutdown,
and shootdown handshakes preserve the existing separation between membership
changes and policy work. Targets above bit 63 must acknowledge retirement just
like lower-index targets; a timeout never releases their resources.

## Scope and verification

The boot suite uses the growing production constructors and a separate native
`scale-probe` ELF. Its parent starts six processes; each child creates twenty
additional threads. The fixture requires seven live processes, 127 live threads,
127 registered timed waits, distinct private TLS, shared state, and kernel stacks
crossing a leaf-table boundary. It then stops and reaps all processes. A separate
injected membership-allocation failure must publish nothing and preserve the
unpublished owner for a successful retry.

Host tests exercise larger tables, stale identities, retained reservations,
cleanup retry, explicit quotas, and an allocator that returns null. The final
configuration matrix and source hashes are recorded in
[scaling evidence](../validation/2026-11-01-scaling/evidence.json).

The shipped runner uses standard MTTCG. Multi-CPU `boot-tests` includes the
shared-root simultaneous-progress and CPU-migration fixture, remote TLB checks,
highest-target missing-ACK retention, wakeup, fault/call ordering, native growth,
and cleanup. A positive 65-CPU guest run crosses the first CPU-set word boundary;
host models cover every configured identity through slot 254. The evidence records
which configurations passed rather than treating the configured maximum as a
verified guest inventory. QEMU results do not establish physical concurrency or
hardware performance.

The fault/call policy fixture uses a boot-only full-batch entry: both real jobs
can return through their existing two-millisecond quantum without the ordinary
early peer-reschedule IPI. The fault program executes UD2 independently of a peer
barrier; its sibling executes an actual native int 0x80. The fixture checks the
captured RIP, terminal states, and suppressed debug output after process death.
Normal production batch scheduling retains its early-reschedule behavior.

This stage retained a static boot frame pool for at most four GiB of Usable RAM
across 288 normalized ranges. [ADR 0013](0013-runtime-frame-metadata.md) replaces
that bitmap-size limit with boot-reserved runtime metadata. Address budgets,
identifier representation, physical memory, and pending cleanup remain finite
limits. BSP policy, linear task and
deadline scans, and whole-page metadata allocation are functional mechanisms;
they do not establish scheduling scalability, hardware performance, or power use.
