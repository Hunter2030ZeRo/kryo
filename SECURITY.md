# Kryo Security Policy

Status: proposed policy; not evidence that every required control is implemented.
SOF means Cryolite Substitute-On-the-Fly. Kryo's security and SOF managers own
only the kernel. A separate Ring 3 manager owns service, driver and filesystem
SOF policy.

## System and scope

Kryo is Cryolite's no_std x86-64-v3 microkernel. HAL supplies hardware mechanisms;
core owns resources and policy; image connects boot resources and execution.
Scope includes kernel entry/return, CPU state, memory and mappings, capability
operations, IPC, scheduling, device authority, DMA lifetime, kernel update
admission and kernel SOF.

Ring 3 callers and their buffers, scalar arguments, manifests and image bytes
are untrusted even when a caller holds a limited capability. A capability
authorizes only its named operation and rights. Ring 3 update policy remains
outside Kryo's manager; failures in Kryo's mediation of those callers remain
inside this policy.

## Responsibilities

- Kernel security manager: kernel-specific release trust, validation policy,
  local activation authorization, rollback protection and admission decisions.
- Kernel SOF manager: kernel transition preparation, CPU/device barriers,
  versioned handoff, ownership transfer, cancellation and recovery coordination.
- Ring 3 SOF manager: service/driver/filesystem provenance, compatibility,
  state migration and publication policy. It does not gain kernel replacement
  authority by managing system updates.

These managers are proposed subsystems, not implemented runtime services.

## Security invariants

1. Validate live handle, object kind, rights, ranges and resource limits before
   mutation or publication. Descriptive names, IDs and addresses confer no rights.
2. Preserve kernel/user isolation and W^X, including writable aliases. Never
   execute an untrusted pointer or restore unchecked privilege/control state.
3. Initialize memory before exposure. Retain backing owners until CPU/TLB/DMA
   retirement completes. Failed cleanup must remain reachable and charged.
4. Preserve first-completion, accepted-delivery and buffer-ownership rules for
   IPC, cancellation, timeout and service failure. Do not blindly replay work
   whose result is uncertain.
5. Keep parsers, verification work, metadata growth and privileged operations
   bounded. Invalid inputs and ordinary resource exhaustion are errors.
6. Require two independent kernel-image admission checks: after download and
   at the SOF attempt. Both checks cover the applicable trust, integrity,
   authorization and compatibility policy. Download success is not activation
   authority.
7. Bind signatures to the complete kernel artifact and signed manifest:
   kernel identity, length/hash, release/security version, required hardware,
   ABI/dependency and state-handoff compatibility. Trust roots come from the
   configured trusted policy, never from the candidate's self-assertion.
8. Acquire an owned immutable candidate before validation. At SOF, revalidate
   current keys/revocations, rollback policy and local capability against those
   bytes. Bind authorization to one attempt and current policy/source-state
   epochs; retain ownership through transfer and prevent substitution.
9. Specify and verify the relation between signed file bytes and final loaded
   code/data, including zeroing and permitted relocations. Do not compare an
   ELF file digest directly with a different loaded-memory representation.
10. Refuse kernel handoff until CPU, entry/interrupt, mapping, object and device
    ownership invariants are established. A Ring 3 manager's assertion alone
    does not prove DMA or IRQ retirement.
11. Preserve user processes and their live native ABI during kernel SOF.
    Reject unsupported ABI/state transitions instead of restarting applications
    while claiming a live substitution.
12. Download verification failure rejects the update. A verification failure
    or technical error during an authorized kernel SOF attempt rejects that
    attempt and requires reboot. Block further kernel SOF attempts until reboot.
13. Distinguish precommit cancellation from recovery after privileged effects.
    Do not claim that retaining an old image makes arbitrary rollback safe.

Hash/signature validation establishes artifact identity and authorized origin
under its trust assumptions. It does not prove kernel behavior, absence of
vulnerabilities or correctness of arbitrary state migration.

## Reportable findings and severity context

Report reachable violations of kernel isolation, capability attenuation,
ownership/retirement, checked entry/return, bounded untrusted-input handling or
kernel update authorization. Include the attacker-controlled input, required
authority, actual path and practical impact.

Examples include unauthorized replacement, accepted stale/revoked metadata,
artifact substitution after verification, stale CPU/DMA access, cross-process
data exposure, duplicate completion or accepted-operation replay. Resource
exhaustion is meaningful when a reachable input defeats the intended bounds.

Separate implemented reachable behavior from a proposed design gap. Tests,
models, emulation, a signature or an unavailable attack path are not blanket
proof that an invariant holds or that a finding can be suppressed.

## Known limitations and unresolved decisions

Current capability, VM, SMP and service-lifetime mechanisms do not implement
kernel SOF. No kernel trust-root, update signature verification, authenticated
rollback floor or complete kernel handoff protocol is currently provided.

Current user-lease quiescence does not stop every CPU in image-independent
transition code. CPU-local tables, stacks, callbacks, kernel heap ownership
and execution metadata can still reference the old image.

Root provisioning/rotation, signature/hash suites and thresholds, freshness
and offline revocation, authenticated time, persistent antirollback storage,
recovery authorization and handoff schemas require explicit design review.
Do not infer accepted risk, a TPM requirement or permission to ignore expiry.
Until required evidence is available, the corresponding SOF profile must deny
admission. No kernel-SOF activation endpoint is introduced by this document.

A compromised running kernel cannot establish a fresh root of trust merely by
validating its own replacement. Boot-chain and hardware trust assumptions must
be stated before any stronger recovery claim.

## Exclusions and accepted risk

No new security finding class is excluded and no new risk is accepted here.
Ring 3 component update semantics belong to their separate manager, while
kernel boundary violations involving those components remain in scope.
