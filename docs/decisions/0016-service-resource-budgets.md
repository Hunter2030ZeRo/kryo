# ADR 0016: Service resource budgets

Status: Accepted

## Authority

Trusted boot creates ResourceBudget accounts with immutable limits. User code
cannot mint accounts or raise limits. ASSIGN authorizes selecting an existing
account for a child; TRANSFER is also required because selection shares authority
with another process. READ permits inspecting an account.

ManifestBudget stores a descriptive source handle in the creator-owned launch
manifest. Spawn revalidates kind, generation and ASSIGN|TRANSFER before cloning
the account. Without explicit selection, manifest and legacy spawn inherit the
parent's account. Threads always share their process account. Restart selects the
same service account and does not reset usage.

Leaf services receive no assignment capability. BudgetSelf exposes their own
limits and usage without reassignment authority. Service labels cannot select an
account. The supervisor retains the explicit account capabilities in its root
startup inventory.

## Charged resources

| Resource | Charge lifetime |
| --- | --- |
| Shared RAM pages | Once per memory payload, through delegated references and mappings |
| Handles | Each open or reserved destination slot |
| Waits | Registration through Parking, Blocked and Ready, until take or abandonment |
| Live threads | Initial and created threads, including preparation and deferred retirement |

Charges retain the account independently of process execution and cannot be
cloned. Reservation checks overflow and available credits before incrementing
usage. Failed unpublished work releases credits after rollback. Handle closure or
terminal process publication does not imply complete resource retirement.

Handle credits belong to actual table slots. Forgetting a reservation token cannot
refund held capacity. IPC charges the destination before acceptance; committed
grants retain those credits. Unused reserved slots refund at commit or release.
Metadata generation history remains separate from live-handle usage.

A shared mapping retains the entire memory payload and its page charge after
creator exit or source handle closure. Final destruction returns backing before
refunding credits. Forgotten arena references retain both. Failed backing return
must not refund leaked storage.

Wait completion alone does not release credit. Accepted capability delivery and
saved-context publication finish before wait consumption. Failed registration
refunds unpublished charges. Close, Reply, Signal, cancellation and inspection
remain usable without a new wait when wait credits are exhausted.

Closing a thread-control handle does not refund its live-thread charge. Initial
process threads also consume credit, preventing spawning from resetting capacity.
Private failed-load and failed-thread owners retain their charge until stacks and
mappings retire. Dropped unretired owners retain their credits.

## VM bounds

Each account defines per-process mapping and private-page-table limits. These
cover aliases and include the root table. They apply before initial image/stack
mapping and during later mapping. Existing bounded/growing constructors remain
available to trusted regression fixtures.

This is not exact physical-RAM accounting. Shared-page usage counts new shared
payloads; private mappings and tables have separate per-process bounds. Kernel
heap pages, HHDM splits, stack-region tables and metadata are not charged byte for
byte. The live-thread account bounds execution owners across process creation.

## CPU and SIMD

Scheduling remains per-thread round-robin with serialized BSP kernel policy.
More runnable threads can increase a service's share within its thread budget.
Preemption counts are diagnostics, not CPU time. CPU budgets need per-job elapsed
accounting on the executing CPU; BSP batch wall time cannot substitute for
independently returning parallel jobs.

User state preservation remains eager x87/SSE and optional AVX with the current
bounded save layout. Kernel code remains soft-float. This change adds neither
kernel SIMD regions nor AVX-512, heterogeneous-CPU feature policy, priority
inheritance or per-service CPU fairness.

## Verification

Host models cover checked charging, shared/independent accounts, reserved handles,
Ready waits, shared-memory lifetime, VM bounds and failure ownership. Guest checks
exercise exhaustion before authority acceptance, inheritance, retained mappings,
live threads after control closure, recovery through nonwaiting operations and
failed cleanup. Dated status and validation artifacts record actual execution.
QEMU results establish functionality, not hardware performance or energy use.
