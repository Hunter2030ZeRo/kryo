# ADR 0015: Service startup and authority distribution

Status: Accepted

## Boundary

The kernel validates and installs capabilities. The user-space supervisor selects
images, service roles, discovery policy and restart behavior. A ServiceId or
instance number describes policy; neither grants authority or authenticates a
message. Modules requiring an isolation boundary run in separate processes.
Threads in one process share its handle table and authority.

Normal boot starts the native supervisor with an explicit root inventory. Its
compiled manifest starts an echo service and two discovery clients. This is the
first configured service graph. Text configuration, dynamic inventory and general
device management remain separate work.

## Startup transaction

ManifestCreate allocates a creator-process-owned LaunchManifest. It contains a
version, nonzero service/instance labels, an argument and role descriptions.
ManifestGrant appends a unique nonzero role, source handle, object kind and
requested rights. The descriptor has READ|WRITE and cannot be duplicated or
delegated. It contains no source object references and preserves no authority
after a source handle closes.

Append checks type, meaningful rights, source TRANSFER and attenuation. SpawnManifest
checks those conditions again against the caller's current generational handles.
It snapshots the plan and acquires every source reference before preparing a
private child. A changed or stale source fails the entire attempt.

The parent reserves result-handle capacity. All child capabilities, its immutable
startup record and process control are installed before the scheduler can dispatch
the child. Installation may use individual reserved slots while the child remains
private; failure destroys its whole unpublished handle table. VM/stack cleanup
that cannot finish remains owned by the parent's pending creation state. No child
can forward partially installed grants before a failed transaction rolls back.

The child receives one READ-only StartupRecord handle in RDI. Its entries describe
exactly the installed child handles. StartupRead returns individual words, so the
initial runtime needs no fixed user mapping or arbitrary user-pointer copy. No
WRITE, MAP, DUP or TRANSFER right is granted on the record. It is an immutable
startup receipt, not proof that a handle remains open indefinitely.

Record version 1 has seven header words and four words per grant, with at most
32 grants. This is a record-format bound, independent of process/thread counts.
Ordinary IPC still carries at most two capabilities; legacy Spawn retains its
two-register startup convention. The experimental syscall ABI remains version 0.

## Role policy

Each service manifest lists required roles, expected object kinds and maximum
rights. Unknown roles, duplicate roles, missing required roles, kind mismatch and
rights outside the role's maximum fail closed. The kernel separately checks the
source capability; a manifest label cannot manufacture rights.

Current default grants are:

| Role | Child authority |
| --- | --- |
| Registration | SEND on one private instance gate |
| Directory | SEND on one private client gateway |
| Channel factory | CREATE on that service's account |
| Debug output | WRITE |
| Bootstrap checkpoint | SEND on a separate diagnostic gate |

DUP and TRANSFER are absent unless a role explicitly needs delegation. The
supervisor retains image EXECUTE authority and process-control handles; ordinary
service clients receive neither. Lookup authority does not include service stop
or restart. Bootstrap checkpoint gates coordinate fixed restart, accepted-call
cancellation and account checks; they are separate from directory traffic.

## Registration and discovery

Receive currently exposes no authenticated sender identity. The supervisor
therefore creates a private registration endpoint for each instance and gives its
SEND capability only to that process. Registration is checked against the service
and generation bound to that gate. A shared public endpoint accepting a claimed
ServiceId would not provide this authentication.

The service advertises an Endpoint with exactly SEND|TRANSFER. HandleInfo reports
the received handle's actual kind and rights; packet metadata is insufficient.
The registry becomes Ready only after that verification. Registration gates close
after admission. A new instance receives a fresh gate and creates a fresh endpoint.

Each client receives a distinct directory gateway. The supervisor binds its policy
principal to that endpoint; the request's claimed identity does not authorize the
lookup. The allowlist checks the trusted principal, target and requested rights.
Unsupported protocols, malformed requests and denied authority produce no grant.

Reply carries no capabilities. A client sends a callback endpoint with SEND alone;
the supervisor uses SendCaps to deliver the selected endpoint with SEND alone.
Control, receiving, duplication and direct capability redelegation are absent.
This limits kernel capability transfer; it does not prevent an authorized process
from proxying requests or sharing information through its own protocol.

## Lifetime and failure

Registry records move through Stopped, Starting, Ready, Stopping and Failed.
Beginning an attempt consumes a checked generation before publication. A failed
attempt does not reuse that generation; wrap is rejected. These generations are
scoped to the current supervisor inventory lifetime, not a durable global epoch.

Withdrawal removes the entry before stopping the process. Endpoint owner death
closes old client capabilities. Replacements never reopen or retarget them.
Already accepted work retains its uncertain-outcome semantics.

The root gate factory and each service's factory have separate channel accounts.
Restart reuses the service's existing account. Retained old payloads stay charged,
including through creator death, until final references retire. One service cannot
reset its account by restarting or consume another factory's channel credits.
Memory, handles, waits and CPU time still require general resource budgets.

Restart is not global descendant revocation. Shared memory and intentionally
delegated factory references retain their existing lifetimes. Device handles also
remain bound to their original ProcessId; copying a register handle does not
rebind hardware authority to a new driver. DMA and device recovery need explicit
quiescence and retirement.

The initial supervisor is a root process. Its own restart, durable epochs and orphan
recovery are not implemented. The configured registry uses manifest-sized storage
without a user allocator; this is not a kernel-wide service limit. Multi-object
waits and a scalable discovery event loop remain future work.

The initial event loop polls private gates with ReceiveFor. Its interval begins
after kernel admission checks, so time spent resuming after a separate ClockRead
cannot consume the interval before receive is admitted. Ordinary RPCs retain
absolute deadlines. The echo fixture discards InvalidReply after an accepted
call ends; it does not turn that per-request outcome into process failure. A
separate server checkpoint holds its acknowledgement call until the supervisor
cancels the known client wait and releases the server. This exercises a late
reply without assuming a short deadline will be accepted under every scheduling
load. Absolute timeout paths remain separately tested.

Separate channel accounts isolate newly created channel charges. They do not
eliminate retained-reference backpressure: a client retaining an old endpoint
continues to charge that service's account. Normal replacement keeps Failed
services unpublished and retries with backoff on the same account. This retry
branch requires separate failure-injection evidence from successful restarts.

## Verification

Host checks cover startup decoding, role policy, generation exhaustion, stale
registration and discovery allowlists. Guest failure fixtures cover source/owner
validation, allocation failures, private installation and post-load publication.
Actual Ring 3 clients exercise forged identity, denied operations, reduced grants,
account separation, service replacement and old-capability failure. Dated status
and validation artifacts record completed runs and their limits.
