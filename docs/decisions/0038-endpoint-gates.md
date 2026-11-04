# Endpoint gates and authority closure

Date: 2026-11-04. Status: implemented experimental native contract.

## Routing and authority

An EndpointGate is an immutable send authority for one receiver-owned Endpoint.
`EndpointGateCreate` (66) takes the root handle in RDI and a nonzero u64 label
in RSI; RDX returns the new gate handle. Minting requires root CONTROL and the
exact receiver-owner task. A label is unique per root while its gate is active
or its closure event awaits delivery. Gates allow SEND, DUP and TRANSFER only.
Copies and capability transfers retain the original object and label. Gates
cannot receive, mint gates, close the root, or increase authority.

The runtime resolves the payload's registry index only after matching the
retained gate ObjectId and label. A stale gate surviving root closure cannot
route to a reused registry slot. Root closure or receiver death discards queued
closure events and retires forwarding records; stale gates return PeerClosed.
A receive completion already committed before closure retains its winning result.

## Receive metadata

Every successful Receive defines RCX as the full-width label and R15 as
`ReceiveEvent`: Message = 0, GateClosed = 1. Ordinary root senders produce
Message with label zero. Gate senders produce Message with their immutable
kernel-stamped label. User message words and outgoing grant-rights registers
cannot choose event identity. These new register outputs apply only to Receive,
ReceiveUntil and ReceiveFor. CallExchange preserves its input RCX reply capacity
and R15 second-grant rights, matching existing compiled callers. Its SDK result
synthesizes label zero and Message instead of decoding those input registers;
response capability ownership is unchanged.

GateClosed contains the label, four zero message words, zero capabilities and
no reply token. Message and event queues alternate when both can progress, so
neither pending authority closure nor ordinary queued messages can starve the
other. There is no global ordering across unrelated authorities.

## Ownership and allocation

Minting reserves a handle, a registry slot, an endpoint event slot and one
credit from the root channel account before publication. The gate payload owns
the credit; the registry retains one internal gate reference until closure
completion consumption or root closure. Stale external gate owners therefore
remain charged after forwarding has stopped. Unpublished failures release all
owners and reservations.

Handles, duplicates and staged IPC or ReplyCaps grants are strong gate owners.
A native gated Send or Call additionally retains a per-thread gate loan through
wait settlement. Source handle closure cannot overtake a queued operation.
The registry's own reference is excluded: a reference count of one marks the
last external owner. The arena Drop path only advances a saturating gate epoch
on the two-to-one transition; it never allocates or enters endpoint policy.

At serialized settlement, a changed epoch triggers a registry scan. Sampling
before scanning preserves cascaded drops for the next pass. Endpoint retirement
explicitly dirties gate settlement. Final process-handle destruction settles
before reap returns, so an unbounded receiver can wake before an idle decision;
cleanup-error returns also settle any unpublished owners already retired.
Saturation stays permanently dirty, with
progress-based fixed-point termination. Unchanged steady IPC and Receive do
not scan the gate registry. Mint and final-reference collection are O(gates);
this first contract makes no fast-path latency or throughput claim.

The closure queue uses a circular FIFO with one tail index. The closure queue
links and record storage are allocated at mint. Final close,
event publication and completion do not allocate. Endpoint storage keeps its
existing inline wait records rather than introducing another boxed payload.
Private reply-capacity storage uses checked u8 values (None or 0..2), while
public arguments remain usize and retain their existing bounds checks. On the
pinned x86-64 build, Endpoint<2,2> remains 312 bytes and KernelObject remains
328 bytes, equal to the preceding source. Completion grows from 72 to 80 bytes;
a gate record is 72 bytes and the gate's payload fields total 24 bytes. Small
variants still occupy the largest enum payload's storage; this change does not
remove that existing per-object padding or establish a performance result.

## Accepted work

GateClosed means that client capabilities, staged grants and native gate waits
have retired. It does not prove completion of accepted service work. In
particular, a caller timeout can end its kernel wait after a request has been
accepted; the reply token then expires, while the service operation may continue.
A service must retain its own operation/state ownership until that work finishes
and defer state destruction when necessary. No file descriptor, filesystem
operation, cancellation acknowledgement or server-side job lifetime is added to
Ring 0 by this contract.

## Validation boundary

Host tests exercise stamped labels, capability-bearing calls, closure ordering,
owner validation, root-death discard, allocation failure and the saturating
epoch. Boot checks exercise registry identity reuse, delegation/source close,
retained credits, publication rollback and unchanged-epoch scan avoidance.
Three native Ring 3 boot cases close a source handle while Send or Call is
queued, then require message-before-closure delivery. They include accepted
Call timeout and rejection of the expired reply token, followed by independent
process, object, wait, heap and backing retirement. A fourth native fixture
holds the last gate handle until client-process reaping and requires the server
to become runnable immediately afterward without deadlines or polling.
A capability exchange regression sends two request grants with reply capacity
two and checks raw RCX/R15 immediately after the response, before another call.
The Rust decoder separately checks non-receive register normalization.
Independent C/Rust Ring 3 fixtures validate the native wire contract separately.
These checks do not establish a file service, BetAFS integration or production
performance bounds.

The C/Rust ABI fixture routes two distinct gates through one receiving thread.
It checks labels across duplication and ReplyCaps, final-reference notification
after source closure, and a retained second gate after receiver death. The C
wrapper also creates a gate on its own endpoint and checks closed-root behavior.
Kernel teardown requires channel, object, quota, heap and frame retirement.

The label mechanism follows conventional capability badging described in the
[seL4 IPC documentation](https://docs.sel4.systems/Tutorials/ipc.html).
Kryo's final-authority event and lifetime contract are separate mechanisms.
See the [validation record](../validation/2026-11-04-gates/README.md).
