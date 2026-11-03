# Interface design

The public service ABI is not stable. Experimental task calls are defined below; the remaining sections list decisions required before publication. Ownership and concurrency rules are specified in the [design contract](design-contract.md).

## Experimental task calls

`interfaces/src/syscall/` defines version 0, used by normal service execution and `boot-tests`. Entry is `int 0x80`; RAX selects a call, RDI and RSI hold arguments. Return RAX is zero or a negative error; RDX holds the result. IPC uses R8..R11 for four u64 words. Receive and successful Call replace those message registers. Receive also replaces R12/R13 with received handles and R14 with their count; other general registers are preserved. The separate result registers preserve all 64 bits of a handle or reply token.

| Number | Call | RDI | RSI | RDX on success |
| --- | --- | --- | --- | --- |
| 0 | Yield | Unused | Unused | 0 |
| 1 | Exit | Exit status | Unused | Does not resume |
| 2 | TaskStatus | Task-control handle | Unused | Task status |
| 3 | TaskStop | Task-control handle | Exit status | 0; self-stop does not resume |
| 4 | Close | Handle | Unused | 0 |
| 5 | Duplicate | Handle | Reduced rights | New handle |
| 6 | Send | Endpoint handle | Unused | 0 after receiver acceptance |
| 7 | Receive | Endpoint handle | Capability capacity, 0..2 | Reply token, or 0 for Send |
| 8 | Call | Endpoint handle | Unused | 0 after reply |
| 9 | Reply | Endpoint handle | Reply token | 0 |
| 10 | EndpointClose | Endpoint handle | Unused | 0 |
| 11 | Wait | Notification handle | Unused | 0 |
| 12 | Signal | Notification handle | Unused | 0 |
| 13 | CancelWait | Task-control handle | Unused | Delivery state |
| 14 | MemoryCreate | Page count, 1..8 | Unused | New memory handle |
| 15 | MemoryMap | Memory handle | Page-aligned VA | 0; input RDX=object page index, R8=0 for RO or 1 for RW |
| 16 | MemoryUnmap | VA in caller's space | Unused | 0 |
| 17 | SendCaps | Endpoint handle | Capability count, 1..2 | 0 after acceptance |
| 18 | CallCaps | Endpoint handle | Capability count, 1..2 | 0 after reply |
| 19 | SetTls | FS base | Unused | 0 |
| 20 | ClockRead | Unused | Unused | Monotonic nanoseconds |
| 21 | SleepUntil | Absolute deadline in nanoseconds | Unused | 0 |
| 22 | SendUntil | Endpoint handle | Unused | As Send; input RDX=absolute deadline |
| 23 | ReceiveUntil | Endpoint handle | Capability capacity, 0..2 | As Receive; input RDX=absolute deadline |
| 24 | CallUntil | Endpoint handle | Unused | As Call; input RDX=absolute deadline |
| 25 | WaitUntil | Notification handle | Unused | As Wait; input RDX=absolute deadline |
| 26 | SendCapsUntil | Endpoint handle | Capability count, 1..2 | As SendCaps; input RDX=absolute deadline |
| 27 | CallCapsUntil | Endpoint handle | Capability count, 1..2 | As CallCaps; input RDX=absolute deadline |
| 28 | Spawn | Image handle | Startup grant count, 0..2 | New process-control handle |
| 29 | ProcessWait | Process-control handle | Unused | Exit code; R8=reason |
| 30 | DebugLog | Debug-console handle | Event word | 0; input R8=value word |
| 31 | ProcessWaitUntil | Process-control handle | Unused | As ProcessWait; input RDX=absolute deadline |
| 32 | ThreadCreate | RX entry address in caller's process | Argument word | New thread-control handle |
| 33 | ThreadExit | Thread exit code | Unused | Does not resume this thread |
| 34 | ThreadJoin | Thread-control handle | Unused | Exit code; R8=reason |
| 35 | ThreadJoinUntil | Thread-control handle | Unused | As ThreadJoin; input RDX=absolute deadline |
| 36 | DeviceRead | Device handle | Register offset | u32 value |
| 37 | DeviceWrite | Device handle | Register offset | 0; input RDX=u32 value |
| 38 | EndpointCreate | Channel-factory handle | Unused | New endpoint handle |
| 39 | NotificationCreate | Channel-factory handle | Unused | New notification handle |
| 40 | ManifestCreate | Startup-record version, 1 | Nonzero service label | New manifest handle; input RDX=instance, R8=argument |
| 41 | ManifestGrant | Manifest handle | Nonzero role | 0; input RDX=source handle, R8=kind, R9=rights |
| 42 | SpawnManifest | Image handle | Manifest handle | New process-control handle |
| 43 | StartupRead | Startup-record handle | Word index | Record word |
| 44 | HandleInfo | Handle | Unused | Wire object kind; R8=actual rights |
| 45 | ReceiveFor | Endpoint handle | Capability capacity, 0..2 | As Receive; input RDX=relative duration in nanoseconds |
| 46 | ManifestBudget | Manifest handle | Resource-budget handle | 0 |
| 47 | BudgetInfo | Resource-budget handle | Resource index | Limit/usage; input RDX=query |
| 48 | BudgetSelf | Resource index | Query | Caller account limit/usage |
| 54 | VmReserve | Byte length | Alignment | Chosen reservation base |
| 55 | VmCommit | Start VA | Byte length | 0 |
| 56 | VmDecommit | Start VA | Byte length | 0 |
| 57 | VmRelease | Exact reservation base | 0 | 0 |
| 58 | Park | Private word VA | u32 expected | 1 on mismatch, 0 after wake |
| 59 | ParkUntil | Private word VA | u32 expected | As Park; input RDX=absolute deadline |
| 60 | Wake | Private word VA | Maximum waiters | Number detached |
| 61 | ThreadCreateNative | Version, 1 | RX entry | New thread-control handle; input RDX=argument |

TaskStatus requires INSPECT; TaskStop and CancelWait require CONTROL. They also accept process-control handles; a retained terminal result continues to report Exited after reaping, and stop/cancel cannot target a replacement generation. Duplicate requires DUP and a subset of the original rights. Rights bits are INSPECT=1, CONTROL=2, DUP=4, SEND=8, RECEIVE=16, WAIT=32, SIGNAL=64, READ=128, WRITE=256, MAP=512, TRANSFER=1024, EXECUTE=2048, CREATE=4096, ASSIGN=8192. Send/Call and their Caps variants require SEND, Receive/Reply require RECEIVE, and EndpointClose requires CONTROL. Wait and Signal require their matching rights. Handles are process-local and opaque; zero is invalid. Close removes one reference, not all authority to an object. A task-control object can outlive its task and report Exited without keeping its VM or stack alive.

EndpointCreate and NotificationCreate require CREATE on a channel factory. The kernel mints factories during trusted startup; neither call creates a new account. Duplication and copy delegation retain the same finite live-channel budget across processes and threads. Exhaustion returns OutOfResources. Creation reserves result-handle capacity before charging or publishing an object; all unpublished state rolls back on failure.

The caller becomes the endpoint receiver/replier or notification waiter. EndpointCreate returns SEND|RECEIVE|CONTROL|DUP|TRANSFER; NotificationCreate returns WAIT|SIGNAL|DUP|TRANSFER. RECEIVE and WAIT remain bound to that exact thread and cannot be delegated. A sibling can use granted SEND/SIGNAL rights but cannot consume for the creator. Creator-thread death closes the channel. A retained stale capability continues to fail with PeerClosed, including after a replacement service starts.

Closing the last external reference also closes the channel and resolves pending waits. The serialized runtime removes closed channel registry roots after committing accepted transfers and publishing completions. A closed channel remains charged while any external or staged reference retains its payload. Only final payload destruction returns budget credit. Device notification roots remain until explicit hardware retirement. This channel count does not budget memory, handles, waiting threads, or queued bytes; general quotas remain pending. [ADR 0014](decisions/0014-native-channel-creation.md) defines the lifetime rules.

Errors are InvalidCall, InvalidHandle, PermissionDenied, WrongType, OutOfResources, InvalidArgument, PeerClosed, Cancelled, InvalidReply, TimedOut, and Busy. Unknown call numbers and rights bits are rejected. MemoryMap validates a VA in the caller's space; private word calls snapshot only validated owned RAM. General arbitrary user-copy recovery remains pending.

SendCaps/CallCaps supply source handles in R12/R13 and requested rights in R14/R15. Every source requires TRANSFER and the requested rights must be a subset. Delegation copies authority; source handles remain valid. RECEIVE and notification WAIT ownership cannot migrate. Receive reserves its requested capacity before publication; insufficient capacity rejects a capability-bearing request before acceptance. Received capabilities install before runnable publication. Reply currently carries inline data only.

MemoryCreate returns READ|WRITE|MAP|DUP|TRANSFER authority to zeroed, non-executable RAM. MemoryMap requires READ|MAP and also WRITE for RW. Page index, VA, permission flag, and capacities are checked. Each mapping retains the whole memory object. Closing every handle does not remove live mappings; MemoryUnmap retires caller access before releasing that mapping's reference. RO is a view permission and does not promise immutable contents while another holder can write.

SetTls updates only the caller's saved FS base. Zero clears it; other accepted values must remain below 2^47. Invalid values leave the old base unchanged. This does not allocate TLS or process an ELF TLS template. The caller must retain a suitable mapping for its data. User FSGSBASE instructions are disabled; the next validated user resume installs the base.

For a ThreadCreateNative thread, SetTls is further restricted to its leased TLS
span; zero cannot clear that lease. Legacy SetTls behavior remains unchanged.

Spawn requires EXECUTE on an immutable boot-image capability. Input R12/R13 contains up to two source handles and R14/R15 their reduced rights; each source requires TRANSFER. It inherits no other handle. The parent retains its sources and receives INSPECT|CONTROL|WAIT|DUP|TRANSFER authority to a new process-control object. The current transfer policy still rejects requested WAIT or RECEIVE rights across all object kinds; process-observation WAIT delegation is deferred, though reduced control/inspection rights may be copied. Capacity, image, mapping, and stack failures return ordinary errors. Unfinished cleanup remains retained and blocks further creation until retirement succeeds. A child is published only after private preparation and reserved result-handle capacity.

Legacy Spawn children enter with RDI/RSI holding the first two startup grant handles (zero for absent grants); RDX bit0 indicates enabled AVX state and RSP is 16-byte aligned. The older init/worker fixtures retain this convention. Manifest children and the normal initial supervisor instead receive the immutable StartupRecord handle in RDI, as defined below. Their assembly entry calls Rust. The original native-smoke fixture keeps its earlier identity/TLS convention. argv/envp/auxv and ELF TLS templates remain pending.

ProcessWait and ProcessWaitUntil require WAIT and return the same retained terminal status to early or late observers: RDX is the exit code, R8 is Exited=0, Fault=1, or Stopped=2. Fault codes use 0x100+vector (255 for a rejected saved context). Self-observation is denied. Timeout/cancellation ends the observation without stopping the target. Terminal observation does not imply all memory, shared references, or device cleanup has finished. The process registry is separate from the thread scheduler. ProcessWait rejects every member of the target process, including sibling threads. Exit, fatal faults, and process-control stop end all member threads; the first process reason wins. Process exit is published only after every member ends execution.

ThreadCreate creates a thread in the caller's process and shares its address space and handle table. The kernel initializes private FP/SIMD state, a guarded 16-KiB kernel entry stack, a guarded 16-KiB user stack, and a zeroed TLS page. It accepts only an entry in an existing RX mapping. The result grants INSPECT|CONTROL|WAIT|DUP|TRANSFER to one thread generation. Capacity and preparation failures publish no thread and preserve unfinished cleanup owners.

Raw thread entry uses RDI=argument, RSI=TLS address, RDX bit0=AVX enabled, RSP aligned to 16, and FS=TLS page. Use an assembly trampoline which CALLs the Rust/C entry. ThreadCreate's libsys wrapper is unsafe because this entry contract and shared-data lifetimes remain the caller's responsibility. Managed stack, guard, and TLS regions cannot be mapped or unmapped through ordinary memory calls. ThreadExit skips stack unwinding and ends only its caller. When the final thread leaves voluntarily, its status becomes the process result.

ThreadJoin and ThreadJoinUntil require WAIT, deny self-join, and retain the first terminal result through task-slot reuse. Timeout or cancellation ends the join without stopping the target. TaskStatus/TaskStop/CancelWait accept thread-control handles to select one thread. Process-control CancelWait rejects an ambiguous request when several members wait. Closing or using a handle in one thread affects the common process table; lookup still holds a strong object reference until the operation ends. WAIT delegation remains deferred.

ThreadCreateNative (61) retains the same handle rights and process sharing,
while borrowing runtime-owned stack/TLS memory. It accepts this version-1 scalar
record in registers, without reading a user descriptor:

| Register | Input |
| --- | --- |
| RDI | Version, 1 |
| RSI | Entry in an existing RX mapping |
| RDX | Argument |
| R8 / R9 | Stack base / bytes |
| R10 / R11 | TLS base / bytes |
| R12 | FS base inside the TLS span |
| R13 | Unmapped lower guard bytes; zero or a multiple of 4096 |
| R14 | Flags, zero |

Stack base/end are 16-byte aligned with at least 16 bytes; the language runtime
must choose an adequate stack size. TLS is byte-granular and supports disjoint
packed blocks on private RW pages. Every covered data page must be published
private RW backing. Stack/TLS/guard ranges must not overlap other execution
leases or legacy managed arenas. A guard requires a page-aligned stack base.
Unknown version/flags, invalid ranges, missing/shared/RO/RX backing and invalid
entry/FS return InvalidArgument. Leased conflicts that reach VM admission return
Busy (-11), before invalidation or any retiring state. Legacy MemoryMap/Unmap
retain their earlier InvalidArgument exclusion for reserved private-VM arenas;
leased private pages outside those arenas can reach the Busy check.
OutOfResources publishes no thread.

The child enters with RDI=argument, RSI=TLS base, RDX=CPU flags, FS installed and
RSP at the stack end without a return address. Use an assembly CALL trampoline.
The kernel creates only the entry stack and fresh FP/SIMD data state. Runtime
code owns TCB construction, environment inheritance, language return and cleanup.
The unsafe libsys wrapper accepts a `ThreadMemory` value describing its spans.

Execution leases prevent conflicting map, unmap, commit, decommit, release and
space close while the thread can run. Actual user execution ends and the lease
is released before ThreadJoin publishes completion. User mappings stay present
after exit/reaping, so the runtime can reuse or retire them after join without
waiting for kernel-stack reclamation. Thread quota follows actual kernel
cleanup. Failed unpublished creation releases the user lease even if kernel
stack cleanup must retry. Closing a control handle does not end execution.
Native TLS/stack word waits remain until explicit cancellation or mapping
retirement; in-place object reuse requires runtime synchronization. See
[ADR 0027](decisions/0027-runtime-owned-thread-memory.md).

Normal execution grows process, thread, handle, object, wait, and VM metadata with available memory. The previous two-process, two-thread-per-process, four-thread and four-wait fixture capacities are not normal runtime limits. Allocation, metadata exhaustion, and address-budget failures return OutOfResources and publish no partial owner. Resource discovery and per-service quotas remain pending. CPU registration currently supports 255 directed xAPIC processors, independently of thread count. The BSP serializes native calls between execution batches. Legacy endpoint RECEIVE and notification WAIT remain bound to the original owner thread; sibling handle access does not transfer ownership. [ADR 0009](decisions/0009-bsp-user-threads.md) defines cleanup, [ADR 0010](decisions/0010-smp-execution.md) defines SMP ownership, and [ADR 0012](decisions/0012-growing-registries.md) defines growth and failure.

DebugLog requires WRITE on the early debug-console capability and copies two inline words to the existing kernel debug output. It dereferences no user pointer. Init receives WRITE|TRANSFER and delegates WRITE alone to its worker. This bootstrap diagnostic facility does not provide a terminal or a general console driver.

The initial native executable test ABI enters at a file-backed RX ELF entry with RSP aligned to 16, RDI holding instance identity, RSI a private TLS page address, and RDX bit0 indicating enabled AVX state. Its assembly entry calls Rust with the required call-stack alignment. This is a bootstrap convention, without argc/argv/envp/auxv or a public runtime ABI.

Send and Call carry R8..R11; Receive returns them. Reply supplies its response in R8..R11 and its endpoint-local token in RSI. Tokens are bound to the receiver and caller wait and are single-use. Each endpoint has one receiver-owner task in this stage; RECEIVE transfer to another process is unsupported.

Cancellation and endpoint closure return delivery state in RDX: 0 means not accepted, 1 means accepted and possibly executed. After acceptance, a failure does not promise rollback. CancelWait only targets an active wait and requires task-control authority. Already completed operations keep their first result; late replies fail. Wait consumes one coalesced pending notification or blocks; repeated signals before consumption do not count separate events.

ClockRead returns nanoseconds since BSP clock initialization, unrelated to UTC. SleepUntil succeeds once the absolute RDI deadline has been observed; past deadlines succeed immediately. The six timed IPC variants take the absolute deadline in RDX and preserve their untimed counterparts' arguments and rights. A deadline already reached at admission fails with TimedOut (-10) before queue publication, even if the object could complete immediately. Untimed calls remain indefinite. Do not extend deadlines across retries.

Timeout, cancellation, and closure return delivery state in RDX: zero before acceptance, one after acceptance. An accepted timeout does not undo work or revoke delivered handles. The first serialized result wins; late replies fail. Deadline sampling happens before user dispatch and after context return, with all due registrations detached before IPC progress. It provides neither a strict kernel-latency guarantee nor a buffer-loan release guarantee. See [ADR 0007](decisions/0007-monotonic-deadlines.md).

The bootstrap `run_user` helper retains its earlier exit-only int 0x80 behavior for the memory/entry regression cases. `run_context` uses the version-0 dispatcher. This mode distinction is internal test machinery and must be removed before a public runtime ABI is published.

DeviceRead (36) takes RDI=device handle and RSI=register offset, returning a u32 in RDX. DeviceWrite (37) additionally takes a u32 value in RDX. They require READ or WRITE rights and an active binding owned by the caller's process. Only complete aligned 32-bit accesses inside granted windows are permitted. These calls grant neither MAP nor DMA authority. The bootstrap IRQ uses the existing WAIT notification call.

## Request paths

### Manifest startup

ManifestCreate accepts record version 1 and nonzero service/instance labels. It
returns READ|WRITE on a creator-process-bound descriptor, without DUP or TRANSFER.
The argument is application metadata. Neither label nor argument grants authority.
ManifestGrant requires WRITE on that descriptor and TRANSFER on the source. It
checks the source's actual kind, meaningful rights and attenuation, rejects duplicate
or zero roles, and appends at most 32 descriptions. RECEIVE and WAIT remain
nondelegable. A descriptor holds numbers, not retained source references.

SpawnManifest requires image EXECUTE and manifest READ. It snapshots and revalidates
all source handles, stages their references, installs the complete private child
table and immutable record, then publishes execution and the parent's control
handle. Failures expose no partially initialized child and preserve pending VM
cleanup. The descriptor can be reused while its sources remain valid.

Manifest children enter with RDI=StartupRecord handle, RSI=0 and RDX architectural
features. StartupRead requires READ and rejects out-of-range indices. Record words
are explicit u64 values, not a Rust structure copied across the ABI:

| Words | Meaning |
| --- | --- |
| 0..6 | Magic `KRYOSTRT` in little-endian u64, version 1, total word count, service label, instance, argument, grant count |
| Each following group of four | Role, wire kind, actual rights, child handle |

At most 32 entries occupy 135 words. The record grants READ alone and is immutable.
The libsys decoder checks magic/version/length/count, nonzero labels/roles, duplicate
roles, kinds, rights and handles. Per-service role allowlists belong in user space.

HandleInfo describes a handle already in the caller's table; it adds no authority
over the target. Wire kinds are TaskControl=1, Memory=2, Endpoint=3, Notification=4,
ProcessControl=5, ThreadControl=6, Image=7, DebugConsole=8, Device=9,
ChannelFactory=10, LaunchManifest=11, StartupRecord=12 and ResourceBudget=13. They are independent of
Rust enum layout. [ADR 0015](decisions/0015-service-startup-and-authority.md) defines
the transaction and discovery trust boundaries.

ManifestBudget requires manifest WRITE and source-budget ASSIGN|TRANSFER. The
manifest stores a descriptive handle and revalidates it at spawn. Without a
selection, both spawn modes inherit the caller's account. Accounts are created
only by trusted startup; no syscall raises limits. Binding happens before initial
thread, VM and handle publication. The version-1 startup record is unchanged.

BudgetInfo requires READ. BudgetSelf inspects the caller's bound account without
granting assignment authority; unbudgeted trusted fixtures return InvalidCall.
Queries are Limit=0 and Used=1. Resource indices are SharedPages=0, Handles=1,
Waits=2, Threads=3, Mappings=4, Tables=5, PrivatePages=6 and ReservedPages=7.
Indices 0..3 and 6..7 report shared-account usage. PrivatePages counts only
explicit private-VM data backing; existing ELF, managed stacks and legacy TLS
are excluded. ReservedPages counts private virtual reservations independently
of their committed backing. Mapping/table limits are per process; BudgetSelf
reports the caller's current mappings and root-inclusive owned tables.
BudgetInfo rejects Used for
those two indices because there is no aggregate counter. Unknown indices/queries
return InvalidArgument. Exhausted allocation returns OutOfResources.

Shared-page charges outlive creator exit and remain through mappings. Handles
include reserved destination slots. Wait charges remain through Ready until
consumption; live-thread charges survive control-handle closure and failed
cleanup. [ADR 0016](decisions/0016-service-resource-budgets.md) describes the limits
and the remaining kernel-memory/CPU accounting boundary.

Use kernel IPC for small control requests and capability transfer. Use registered shared memory and bounded request/completion queues for bulk data. Measure registration, mapping, copying, and wakeup costs before selecting a path for a workload.

## Explicit private VM

Calls 54..57 operate on reservations owned by the caller's process, shared by
its threads. They return no transferable memory handle and provide anonymous
RW/NX pages only. VmReserve requires a bound account, a nonzero byte length
divisible by 4096 and power-of-two alignment of at least 4096. The kernel chooses
first-fit placement in `[0x1_0000_0000, 0x2000_0000_0000)` and charges reserved
virtual pages. Reserve creates no accessible data pages and allocates no data
frames. Ordinary MemoryMap/MemoryUnmap cannot change the reserved range.

VmCommit requires an aligned, nonempty subrange entirely inside one reservation
with no committed pages. It obtains real RAM, completes zeroing and publishes
private mappings before successful return. Data backing need not be physically
contiguous. Malformed ranges, overflow, overlap and unreserved operations return
InvalidArgument; quota, metadata, mapping/table capacity and RAM exhaustion
return OutOfResources. A normal failed commit leaves existing contents and
private/reservation charges unchanged. Incomplete rollback or retirement quarantines the entire
process and retains owners/charges for teardown instead of resuming partial
state.

VmDecommit accepts aligned nonempty subranges within one reservation, including
uncommitted holes. It removes live data backing and refunds private-page credit
only after access and TLB retirement complete. The VA remains reserved and a
later commit supplies zeroed RAM. VmRelease requires the exact reservation base
and RSI=0. It retires any remaining committed pages before returning the
reservation credit. The runtime must stop all accesses before either operation;
an address does not keep a Rust borrow or concurrent thread access valid.
The Rust libsys decommit/release wrappers are unsafe for this reason.

Intermediate page-table frames remain owned until full process-space close;
decommit/release do not prune them. The per-process table limit still bounds
these retained paths. PrivatePages/ReservedPages therefore do not measure all
RAM retained by a process. The 1..8-page MemoryCreate interface remains a separate
shared-object contract. No demand paging, allocator, protection transitions,
file-backed mappings are implemented by these calls.
[ADR 0026](decisions/0026-private-vm-and-atomic-waits.md) records the accepted
memory and private-wait contracts.

## Atomic private word waits

Experimental calls 58..60 use one four-byte aligned native `u32` in a published,
owned private RW mapping in the caller's address space. ELF data/BSS, managed
stack/TLS and committed private VM are supported. Shared backing, RO/RX pages,
reserved holes and unpublished mappings are rejected. The kernel copies a
validated word through its owned RAM mapping while users are quiescent; no raw
user pointer is borrowed as Rust data.

| Call | Registers | Successful result in RDX |
| --- | --- | --- |
| Park (58) | RDI=word VA, RSI=u32 expected | 1 on value mismatch; 0 after a wake |
| ParkUntil (59) | RDI=word VA, RSI=u32 expected, RDX=absolute monotonic nanosecond deadline | The same results as Park |
| Wake (60) | RDI=word VA, RSI=maximum waiters | Number detached, up to maximum; zero detaches none |

Invalid addresses, alignment, backing or expected values above `u32::MAX`
return InvalidArgument. Metadata or Waits-account exhaustion returns
OutOfResources. A value mismatch returns immediately without queue metadata or
a wait charge. When the value matches, an already due deadline returns TimedOut.
Incoming native calls sample and detach due deadlines before request progress,
including after earlier long-running calls in the same execution batch. This
is sampled ordering, not a guarantee of exact wakeup latency.

The matching compare and queue publication are serialized with wake and mapping
retirement. Word-queue capacity is prepared only for an actual word wait,
before the persistent wait-table registration is charged and published. Pending
and Ready registrations retain their Waits charge until completion consumption
or abandonment. Timeout, cancellation, wake and task death remove queue records
before completing or abandoning the wait; saved return values precede runnable
publication. The first completion wins.

Wake does not write the word, acquire a user lock or buffer a signal for a later
park. The runtime Release-stores its predicate before calling Wake, then loops
around Acquire loads and park results. Callers must treat ValueChanged, Woken,
TimedOut and Cancelled according to that predicate rather than assume a lock
was acquired.

Keys include the unique address space, a monotonic mapping lifetime and the
exact word address. Reusing a VA cannot wake an old registration. Keys do not
retain backing while asleep. Admitted decommit/release, legacy private-page
unmap, managed stack/TLS cleanup, retained thread-creation cleanup retries and
final process cleanup detach affected
registrations before retiring storage. Detached live waiters return Cancelled;
invalidated storage must not be accessed merely because park returned. Failed
VM validation or metadata preparation leaves wait registrations intact.

These calls are process-private. They implement no shared wait, requeue,
priority inheritance, robust-mutex recovery, pthread API or libc/std port.
Word queues and deadline selection remain linear scans under BSP policy.

## Mediated VirtIO calls

Experimental calls 49..53 provide process-bound Device transport information,
start, submit, poll and reset. CONTROL is required for start/reset; WRITE submits,
and READ queries/polls. Submission also requires a READ+WRITE memory handle and
at most three 16-byte little-endian records (offset u64, length u32, flags u32).
Flag 1 denotes device-writable bytes. These are memory-object offsets, never
pointers, physical addresses or hardware descriptor links.

The transport snapshots a private bounded plan and queue. One opaque ticket owns
the accepted chain and original memory reference. Poll returns zero pending or
bit 63 plus a u32 used length once, then rejects the consumed ticket. The Ring 3
driver interprets device status and full transfer length. Submit errors after
publication can be uncertain; reset before recovery rather than assuming no work
was accepted. Reset prepares a new negotiation phase after hardware stops, while
ticket generations continue. Closing a handle alone does not retire process-bound
hardware. The [transport contract](decisions/0018-mediated-virtio-block-transport.md)
defines all registers, feature phases and owner-retention rules.

No VFS/file ABI or native standard-library port is established by these calls.
The [runtime direction](decisions/0019-native-runtime-and-standard-libraries.md)
keeps the common native service representations independent of C/C++/Rust.

## Block service version 1

The initial service uses four u64 words and at most one memory capability for
one 4096-byte block. Gate capabilities enforce read/write/control authority.
Replies distinguish malformed, stale, range, denied, read-only, I/O, uncertain
and unavailable states. Request IDs correlate replies without deduplication.
Cancellation is not a loan-return acknowledgement; the single-threaded instance
accepts a subsequent Info request as a sequencing barrier after cleanup.
[ADR 0020](decisions/0020-initial-block-service.md) defines the records and lifetime.

## ABI decisions

| Area | Required definition |
| --- | --- |
| Representation | Field widths, alignment, byte order, versioning, lengths, and validation |
| Handles | Object types, rights, lookup, duplication, transfer, close, generations, and reuse |
| Shared buffers | Object identity, offset, length, rights, loan duration, and bounds checks |
| Queues | Producer/consumer count, capacity, full-queue behavior, ordering, and notification |
| Async requests | Request IDs, completion, cancellation, timeouts, retry, and duplicate handling |
| Services | Stable identity, instance generation, failure, and reconnection |
| DMA | Pinning, IOMMU mappings, device quiescence, revocation, and late completion |
| Files and paging | VFS handle lifetime, file-backed pages, dirty data, flush, and service failure |

Use explicit wire representations. Do not pass Rust references or trait objects across an ABI. Snapshot validated shared metadata before using it.

The control path currently implements 32 inline bytes and two delegated capabilities. Discovery, linear move semantics, reply capability transfer, quotas, and a stable register layout still require definition before ABI publication.

ReceiveFor uses the same RECEIVE authority, owner and capability-capacity rules
as Receive. After validating the handle, the kernel samples its clock and adds
the input duration with checked arithmetic. Zero duration returns TimedOut;
overflow returns InvalidArgument. This admission-based interval is suitable for
the initial supervisor's bounded polling. It does not replace absolute request
deadlines: ReceiveUntil and CallUntil preserve their existing end-to-end budgets.

## Replacement and recovery

Orderly replacement stops admission, resolves in-flight work, quiesces devices when needed, transfers state, grants authority to the new generation, and reconnects clients.

Crash recovery first establishes instance termination and resource/device safety. It then restores available state and starts a new generation. Accepted requests may have uncertain outcomes; retries require defined idempotence or deduplication.

Kernel mechanisms provide handles, mappings, IRQ authority, waits, and termination. Service protocols define operation-specific recovery.

## Measurements

Compare direct calls, small IPC, and shared queues. Record p50/p99 latency, cycles per request, bytes copied, memory use, context switches, wakeups, idle CPU, energy per operation, recovery time, and lost/retried requests.

Keep QEMU functional results separate from hardware performance and power measurements.
