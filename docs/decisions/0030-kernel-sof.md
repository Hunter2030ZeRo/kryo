# 0030: Kernel SOF admission and handoff

Status: design proposal; no downloader, verifier or kernel-switch implementation.
SOF is Cryolite Substitute-On-the-Fly. This document specifies only Kryo's
kernel profile. Ring 3 service, driver and filesystem SOF belongs to a separate
Ring 3 daemon/manager.

## Ownership and placement

The proposed kernel security manager owns kernel-image admission and trust
policy. The proposed kernel SOF manager owns the kernel transition lifecycle.
Core owns policy and state; HAL supplies CPU/entry/TLB/device mechanisms; image
wires boot trust inputs and the transition path. Networking and package fetching
stay in Ring 3. No source module or stable syscall is selected by this draft.

An I/O client may fetch a kernel candidate or request inspection without gaining
kernel activation authority. The Ring 3 service SOF manager neither approves
kernel images nor owns kernel handoff policy. Generic capability, mapping, IPC
and DMA lifetime enforcement remains Kryo's responsibility.

## Signed admission statement

Define a bounded canonical manifest and envelope before implementing parsing.
Reject duplicates, unknown required fields, unsupported algorithms, invalid
lengths, arithmetic overflow and ambiguous encodings.

The signed statement binds:
- Component identity and type exactly identifying Kryo kernel, release identity,
  complete artifact length and cryptographic digest.
- Kernel-specific signer role/key policy, manifest format and policy version.
- Release ordering and security epoch, source/target constraints and dependencies.
- x86-64-v3 requirements and the admitted CPU/xstate execution profile.
- Native ABI semantics/fingerprint and supported live-user ABI set.
- Export/import handoff schema and supported source-state versions.
- Loading/relocation rules, SOF entry and all executable migration payloads.
- Freshness/revocation evidence required by the selected policy.

Matching a display version or the current experimental native ABI number zero
does not establish compatibility. Do not freeze Rust object layout, Arc
internals, vtables or allocator metadata as an implicit handoff format.

Algorithms, thresholds, trusted key provisioning and persistence are review
decisions. The package cannot authorize its own root or recovery exception.

## Gate 1: after download

Treat transport, cache, filename and fetched metadata as untrusted inputs.
Perform bounded retrieval and acquire an owned immutable candidate before
validation. Validate the applicable trusted metadata chain, kernel-specific
signing authority, current revocation/freshness policy, manifest binding,
length/hash, release/security version and local staging authorization. Check
platform, native ABI/dependencies and declared source-state/handoff compatibility
against the then-current running kernel. Gate 2 repeats these checks against
its current state; Gate 1 does not reserve a future transition.

Publish a candidate only after success. Quarantine/reject incomplete or invalid
candidates without changing the running kernel or rollback floor. An inspection
receipt records what was checked; it is not a transferable activation permit.

Kryo-specific validation policy owns this gate even if a Ring 3 fetch client
provides transport. Ring 3 service update approval cannot substitute for it.

## Gate 2: at the kernel SOF attempt

Freshly validate the kernel-activation capability. Acquire or retain an owned
immutable snapshot before hashing, signature verification or loading. Validate
that candidate again under the current trusted policy. Recompute integrity and
verify signature/authorization independently of the Gate 1 receipt. Check
revocation/freshness, security floor, platform, live user ABI/dependencies and
source-state/handoff support again.

Keep the snapshot immutable and retain it and prepared load destinations through
transfer.
Prevent writable aliases, replacement by path, DMA writes and source recycling.
Only the admitted loader may populate private non-executable destinations; seal
them before executable publication. Validate the file-to-memory transformation,
BSS and allowed relocations. Bind the admitted candidate to a single attempt,
policy epoch, running-kernel identity and source-state generation.

Arrange expensive checks before the bounded user pause where safe. The final
barrier must recheck the bound epochs/conditions before privileged transfer.
A failed check or invalidated bound condition rejects the SOF attempt and
requires reboot; do not retry Gate 2 in place. An image that can still change
is not admitted.

These are two full artifact-admission points, not a prohibition on additional
format, lifetime or state checks within the transition. Independence requires
a fresh decision on the owned bytes under the applicable policy; it does not
require two separate cryptographic implementations.

## Transition lifecycle

Candidate acquisition -> Gate 1 -> retained candidate -> authorized SOF attempt
-> private preparation -> Gate 2 -> bounded quiescence -> validated handoff
-> authority transfer -> health confirmation -> old-resource retirement.

This ordering is a reviewable lifecycle, not an executable protocol. No path may
run arbitrary candidate Ring 0 code before its admission and effect boundary
are satisfied. Every phase has a bounded operation budget and retained owner.

Required handoff conditions include:
- Prepare transition storage and recovery prerequisites before quiescence. The
  transition must not depend on suspended Ring 3 services, blocked IRQ delivery
  or locks held by stopped CPUs. Any separately runnable trusted dependency must
  have an explicit execution and lifetime contract.
- Every CPU is acknowledged in a transition path with stacks and entry handlers
  whose lifetime is independent of the retiring image. Empty user leases alone
  are insufficient; old AP mailbox/HLT code still executes the old kernel.
- NMI, exceptions, IRQ/timer entry, IDT/GDT/TSS/IST, CR3 and FP/SIMD contexts remain
  valid throughout the transition. CLI alone does not exclude every entry path.
- User register state, mappings, TLS, capability IDs/generations, IPC/wait
  completions, quotas and the monotonic deadline interpretation survive.
- Kernel objects, callbacks, heap allocations and their destruction semantics
  have explicit owners and versioned export/import rules. No stale code or
  allocator pointer becomes a presumed compatible new-kernel pointer.
- Transfer physical-frame allocation metadata, page-table owners and shared
  kernel branches exactly once. Do not recreate free memory from the Limine map
  or infer allocation authority from physical addresses. The transition must not
  allocate through the old mutable allocator after freezing its state.
- In-flight I/O/loans and device ownership are fenced, drained or transferred by
  a defined protocol. Kryo verifies its own retirement conditions; an update
  daemon's claim is not proof that hardware stopped.
- New entry paths and root mappings are installed before user execution resumes.
  Old code/data is reclaimed only after all CPU/TLB/callback/DMA references retire.

Preserving a user process does not promise zero pause. Admission, pause budget,
deadline expiry and recovery behavior must be tested separately from throughput.

## Failure, reboot and recovery

| Failure point | Required outcome |
| --- | --- |
| Download verification | Reject the update; do not activate the candidate |
| Verification or technical error during authorized kernel SOF | Reject the attempt and require reboot |

An authorized attempt begins only after the kernel validates the caller's
activation authority and accepts the attempt. Unprivileged or malformed requests
rejected before this boundary cannot set the reboot requirement. Every failure
after acceptance, including preparation or allocation failure, makes
RebootRequired sticky until reboot and blocks further kernel SOF attempts.
This is a failure policy, not an instruction to reset the machine automatically.

Before privileged effects, keep the old kernel running only while its safety
invariants remain established, still reporting reboot required. After privileged
effects, do not promise that the old kernel or applications can safely resume.
Use the established emergency/recovery path and report reboot required without
executing a rejected image. Reboot never authorizes that image or bypasses
normal boot verification and rollback policy.

## Commit and retained ownership

Do not define commit merely as a final jump: calling arbitrary new Ring 0 code
already grants privileged effects. Any earlier import code must be narrowly
specified, trusted and prevented from mutating authoritative state outside its
reviewed contract.

Before effects, cancellation can retain the old kernel and release only safely
retired candidate resources. Quiescing/resetting a device can already change
request outcomes; do not promise a byte-for-byte original state after abort.

After effects, returning to old text is not a general rollback mechanism.
An inverse migration or compatible recovery image must account for changed
objects, device state and accepted I/O. Do not replay uncertain writes.

Coordinate persistent update state, security-floor advancement and a usable
authorized recovery path across crashes. A normal disk file alone does not
prove resistance to disk rollback. Recovery must not silently lower the
security floor or trust a revoked kernel.

## Required tests before activation

Separate pure parser/policy models, cryptographic test vectors, fault injection,
QEMU live transitions and real hardware/device evidence.

Test changed bytes after download; forged/self-authorized metadata; wrong signer/
target/capability; revoked keys; stale/expired metadata; rollback; ABI/state/ISA
mismatch; failed allocation; malformed handoff; missing CPU acknowledgement;
NMI/IRQ entry; pending IPC/deadlines; DMA/reset failure; interrupted commit and
post-effect recovery. Check that failures preserve reachable owners and do not
publish a candidate or falsely repeat an accepted operation. Verify download
rejection separately from sticky RebootRequired, retry denial after an accepted
SOF failure, and rejection of unauthorized requests without changing reboot state.

A complete SOF trial must preserve running user applications, handles, memory,
threads and their results across an actual kernel replacement. Current boot,
SMP quiescence and service restart tests do not establish this property.

## Current blockers

Kryo currently has no update trust chain or kernel SOF implementation. Existing
AP parking still references old-image code/stacks and CPU-local tables. The
kernel has monotonic time but no established trusted wall-time/freshness source.
Antirollback/recovery persistence, crypto policy, handoff schema and transition
island remain unresolved. Production activation remains unavailable until the
selected profile supplies these guarantees.

## References

- TUF v1.0.37: https://theupdateframework.github.io/specification/latest/
- Linux livepatch consistency: https://docs.kernel.org/livepatch/livepatch.html

These are design references, not claims of TUF compliance or reuse of Linux's
implementation. Live function patching is not whole-kernel substitution.
