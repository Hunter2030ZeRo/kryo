# Static provider threads

Date: 2026-11-04. Status: implemented static provider extension.

Extend the static ANSI mlibc profile with C11 thread creation, join and detach.
Reuse upstream mutexes, conditions, once state, TLS initialization and language
cleanup. Cryolite supplies native creation and the ownership/completion adapter.
No Linux clone ABI, new kernel call or public pthread representation is added.
POSIX attributes, cancellation, dynamic loading and dynamic TLS remain disabled.

## Storage and publication

Each provider TCB has a dedicated private reservation. The provider computes
the static TLS/TCB size and alignment, and placement-constructs its actual TCB,
local-key array and DTV in the supplied storage. The port owns a metadata prefix
and the complete page-aligned payload span. It never rounds a malloc pointer
outward to borrow neighboring objects. The original Variant II layout and ELF
TLS template initialization remain upstream code.

The static profile does not enroll these TCBs in the loader's infallible dynamic
TLS vector. Its native descriptor registry owns them instead. Dynamic/POSIX
builds fail the backend's compile-time gate until that boundary is implemented.
Initial TCB failure stops process startup; child TCB/storage failure returns an
ordinary creation error. PrepareStack and native Clone failures release all
unpublished owners. Default provider stack size and a lower guard are retained;
stack pages are explicitly committed rather than fault-grown.

Native control handles remain full-width capabilities. Provider lock-owner IDs
use the provider's positive 30-bit field and a process-local registry, retaining
each ID through actual retirement. A wrapped cursor searches only unused IDs;
there is no cumulative creation limit. The initial thread uses ID 1. Its
startup grants contain no native self-control, so initial-thread join/detach
remain unsupported. Ordinary nonrobust lock-owner death is not recovered.

The creator completes the descriptor and native handle before publishing the
TCB's thread ID with Release ordering. The child waits with Acquire before
entering provider or application code. A creator pin extends storage lifetime
through the final publication wake, including a child that immediately
self-detaches and exits. The creator clears that pin and wakes observers under
the registry lock before dropping its final access.

The port captures and restores the creator's floating-point environment before
the child invokes user code. This follows [C11 section 7.6](https://open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf).
Kernel soft-float compilation and eager user xstate preservation stay unchanged.

## Completion and detach

Upstream exit runs TLS and TSS destructors, publishes its result and didExit,
then calls native ThreadExit. DidExit alone cannot authorize backing release.
Join claims a descriptor before dereferencing its TCB, drops the registry lock,
and obtains successful native completion. It then waits for the creator pin,
copies the result and closes/releases the control, stack, TCB and ID. Incomplete
cleanup retains its descriptor for retry; unproved completion never frees it.

Detach starts one process-local raw reaper before committing the transition.
Creation failure leaves the target joinable. The reaper scans detached records
whose language cleanup has finished, claims one, and performs the same native
join and retirement. It skips records still executing language cleanup and
never frees its own stack or uses a fabricated provider TCB. Its guarded stack, dummy FS,
control handle and idle wait last until process exit. It uses only native
helpers and already completed objects' trivial destruction.

When the final application/provider thread exits, the runtime calls provider
exit(0), allowing atexit and global destructors to run before process termination.
The raw reaper does not keep a finished application alive. This follows
[C11 section 7.26.5.5](https://open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf).
TSS values are cleared before their destructor callback, preserving values
reinstalled for another iteration, as clarified by
[WG14 DR 0416](https://open-std.org/JTC1/SC22/WG14/issues/c11c17/log.html#issue0416).

Native wait-credit exhaustion yields and returns a spurious futex wake so
provider predicates can retry. EAGAIN still means an actual changed word.
Already completed native joins return their retained result without acquiring
a wait credit; deadline admission and capability checks retain their order.
Retained process results use the same immediate path after target/self checks.

## Verification boundary

The optional libc probe exercises provider TLS/errno isolation, inherited FP
rounding, contended mutex/condition/once operations, TSS cleanup, joined return
values, detach before/after language exit, self-detach, publication failure and
repeated backing reuse. A second process exits through thrd_exit as the final
provider thread and must still emit atexit/destructor events. A third has only
one wait credit and must observe the fallback while completing all work.

Successful fast self-detach does not force a particular instruction interleaving
between child exit and creator publication; the pin/registry proof is separately
reviewed. These tests do not establish complete pthread/POSIX support, dynamic
TLS, signals, robust mutexes, hardened entropy/startup, or hardware latency.

See the [validation record](../validation/2026-11-04-provider-threads/README.md)
for tested source hashes, native wait checks, builds and QEMU cases.
