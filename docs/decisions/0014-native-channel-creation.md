# ADR 0014: Native channel creation

Status: Accepted

## Context

Native code could send, receive, call and reply through channels constructed by trusted kernel fixtures. It could not establish its own service endpoint. The runtime also kept a strong registration reference after channel closure. Exposing creation without retirement would retain every created payload for the runtime's lifetime.

## Authority and accounting

A trusted startup path mints a ChannelFactory containing a finite ChannelBudget. CREATE authorizes EndpointCreate and NotificationCreate. DUP and TRANSFER retain their existing attenuation rules. A duplicated or delegated factory shares its original account; spawning a process or thread cannot reset it.

The account uses shared ownership independent of any process execution lifetime. A successful reservation produces one non-cloneable ChannelLease. The channel payload retains that lease until final destruction. Closing the channel stops admission but does not refund its charge while another handle, staged transfer or trusted root retains the payload. Dropping all factory handles also leaves existing charges intact.

The account is allocated during trusted startup. User creation reserves the existing account without allocating a new one. Runtime handle, arena and registry growth remains fallible. The budget counts live channel payloads only. Wait slots, message storage, memory, handles and execution need separate accounting before claiming general service quotas.

## Creation transaction

1. Validate the factory handle, object kind and CREATE right.
2. Reserve result-handle capacity.
3. Reserve one account charge and create the channel with the calling TaskId as consumer owner.
4. Register one retained channel reference.
5. Commit the original reference into the reserved handle slot.

Failures release every unpublished owner and handle reservation. Capacity failure is OutOfResources. Neither endpoint nor notification creation publishes a partial handle or runnable task.

Endpoints use growing pending/reply storage. The account limit is startup policy rather than a compile-time registry capacity. Existing bounded constructors remain available to failure fixtures.

## Retirement

Settlement commits accepted capability grants before sender/death cleanup, publishes completions, then sweeps registered channels. It closes channels whose only strong reference is the registry and removes closed registry roots outside the arena callback. Closing may produce new completions; settlement repeats until no further registry roots retire.

Pending wait IDs do not themselves retain a channel capability. Once no authority can name a live channel, closing it resolves those stranded operations with PeerClosed. Accepted delivery remains accepted; closure does not undo a committed grant or imply that a request was never executed.

Device bindings retain explicit roots until hardware quiescence succeeds. Automatic channel sweeping excludes those roots. It does not perform hardware work in Drop.

## Consumer and service identity

Endpoint RECEIVE/Reply and notification WAIT belong to the exact creating TaskId. RECEIVE and WAIT cannot cross copy delegation. A sibling does not acquire consumer ownership through its shared handle table. Owner-thread death closes the channel even if other members of the process remain alive.

A service replacement creates a fresh endpoint. Retained old endpoints stay closed and keep their old identity. The bootstrap supervisor checks its own instance words and reconnects through a newly advertised SEND capability. Stable service identities, a registry, versioned startup manifests and a general supervisor remain separate work.

## Validation

Host tests cover shared exhaustion, charge rollback, delegated/stale references, creator destruction and factory attenuation. Guest checks exercise handle/arena/registry publication failure, pending waits, accepted grants, registry-only channels and device-root retention.

The native service probe starts two successive Ring 3 peers, exchanges delegated endpoint authority and request/reply messages, checks shared budget exhaustion while a peer is alive, and verifies stale endpoint failure after exit. It also checks notification signal/timeout and denied authority. The probe runs before the existing init in both normal and boot-test modes and retires its processes, channels and physical allocations completely.

Current run evidence belongs in status and the dated validation directory. QEMU establishes functional behavior; hardware latency, power, scale and device recovery require separate evidence.
