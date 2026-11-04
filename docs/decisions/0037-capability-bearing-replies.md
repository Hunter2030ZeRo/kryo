# Capability-bearing replies

Date: 2026-11-04. Status: implemented experimental native extension.

Return up to two reduced-rights capabilities with the existing four-word reply.
This provides a native IPC mechanism; it defines no file descriptor, VFS or
stream protocol. Existing Call, CallCaps and Reply retain their wire behavior.

## Admission and registers

| Call | Number | Inputs |
| --- | --- | --- |
| CallExchange | 63 | RDI endpoint, RSI request-grant count 0..2, RCX reply capacity 0..2 |
| CallExchangeUntil | 64 | Same inputs, RDX absolute monotonic deadline |
| ReplyCaps | 65 | RDI endpoint, RSI reply token, RDX reply-grant count 1..2 |

All three use R8..R11 for the four message words. Outgoing grants use R12/R13
for source handles and R14/R15 for reduced rights. Zero-grant responses use
ordinary Reply. Source handles remain valid after copy delegation.

An exchange response returns RAX status, RDX zero on success, R8..R11 message,
R12/R13 installed handles and R14 count. A failure preserves the existing RDX
delivery flag: zero before request acceptance, one after possible service-side
work. A reply token and a handle retain all 64 bits. CallExchange returns no
token; the Rust wrapper uses Received with reply always None.

The caller reserves reply capacity and its handle quota before publishing the
wait. Request grants are snapshotted first so a rejected source cannot leak a
reservation. Metadata, wait-credit or handle-quota failure releases unpublished
owners and leaves the request unaccepted. Even capacity zero has an explicit
reservation/completion path. Old calls admit no capability-bearing response.

## Reply transaction

Endpoint preflight checks owner, open state, token generation, pending caller
wait and admitted reply capacity. Source lookup then checks TRANSFER and the
requested rights subset and acquires strong references. Capacity, source and
rights failures leave a still-live token available for retry, including a plain
Reply. Expiration, cancellation or closure can invalidate that token normally.

Only successful preflight and staging allow the endpoint to consume its
single-use token and publish a typed RepliedCaps completion. The image retains
the staged batch under the caller's wait ID in storage reserved at admission.
This publication does not allocate. A server cannot run with the request's
token before request grants have committed, so the earlier request batch has
already left that wait ID.

Settlement installs the response batch into the caller's reserved slots before
publishing message registers, waking the caller or cleaning up a dying process.
Reserved capacity and retained references make that handle commit infallible
under the current serialized ownership contract. A zero-grant Reply refunds
the unused reservation. Timeout, cancellation, death and failed admission use
the existing reservation-release paths. Accepted replies remain committed if
death or cancellation is processed afterward; no user instruction resumes with
part of a response installed.

The kernel still serializes policy after all user execution leases end. This
change does not establish parallel kernel policy or weaken mapping lifetimes.

## Verification

Core tests cover zero/one/two reply slots, retry after insufficient capacity,
legacy completion behavior, token reuse, stale waits, cancellation, timeout,
death, closure and allocation failure. Successful reply publication is also
checked with allocation disabled. Rust decoder tests cover full-width handles,
absent response tokens, excess counts and zero handles.

Six real Ring 3 fixtures use independent address spaces. They cover simultaneous
request and two-capability response delegation, one-capability and plain
responses, rights and source failures followed by a successful retry, handle
quota rollback/recovery, expired admission, accepted timeout and late reply
rejection. Received memory is mapped RO and read; source and recipient mappings,
handles, waits, budget credits and backing retire afterward. Logical heap usage
and physical ownership balance separately. These are model/emulator checks,
not a native filesystem integration or hardware performance measurement.

The C11/C++23 shim uses the same register contract, including the caller's RCX
reply capacity. Compiler checks include O0/O2 and PIC/frame-pointer builds with
no undefined helper symbols. The optional mixed ABI fixture sends a request
memory grant from C to Rust, then receives a separate read-only memory object
and SEND-only endpoint in the same reply. It checks both reduced grants and
reads the returned page before releasing them. Native completion, actual
process reaping and full account/object/backing balance remain separate checks.

See the [validation record](../validation/2026-11-04-reply-caps/README.md) for
source hashes, host checks and the final combined QEMU matrix.
