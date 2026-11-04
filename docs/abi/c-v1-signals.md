# Candidate C-v1 signal view

Status: proposed layouts and registries, not installed headers or working signal
delivery. This extends the [C-v1 foundation](c-v1.md). The native kernel ABI does
not gain POSIX signal calls through these declarations.

## Signal set and identifiers

Use an eight-byte, eight-aligned `sigset_t` containing `unsigned long __sig[1]`.
Signal number n uses bit n-1, including bit 63 for signal 64. Zero is the null
signal used by checking interfaces, never a set member. Set operations accept
positions 1..64; delivery interfaces separately check the assigned signal and
the caller's authority. Reserved positions confer no right to signal a task.

This size avoids copying a 128-byte mask for a 64-position namespace. The
evaluated mlibc generic set implementation derives capacity from sizeof and
requires __sig at offset zero; its bit operations need no layout patch for
this representation. Its public pthread attributes embed sigset_t, so their
eventual Cryolite layout must use this same eight-byte definition. This does
not establish that all provider signal or pthread code works unchanged.

The pinned sigaddset/sigdelset/sigismember implementations subtract one from a
signed signal number before validation. INT_MIN therefore needs a provider
correction or prevalidation before that subtraction. Test INT_MIN, negative,
zero, 64 and 65 inputs before adopting those routines; layout compatibility
does not establish their invalid-input behavior.

The candidate numbers retain familiar Unix assignments where no semantic
difference is intended. Cryolite owns this registry; no Linux header is
imported and no Linux syscall number or signal frame is implied.

| Name | Value | Name | Value |
| --- | --- | --- | --- |
| SIGHUP | 1 | SIGINT | 2 |
| SIGQUIT | 3 | SIGILL | 4 |
| SIGTRAP | 5 | SIGABRT | 6 |
| SIGBUS | 7 | SIGFPE | 8 |
| SIGKILL | 9 | SIGUSR1 | 10 |
| SIGSEGV | 11 | SIGUSR2 | 12 |
| SIGPIPE | 13 | SIGALRM | 14 |
| SIGTERM | 15 | reserved, no legacy name | 16 |
| SIGCHLD | 17 | SIGCONT | 18 |
| SIGSTOP | 19 | SIGTSTP | 20 |
| SIGTTIN | 21 | SIGTTOU | 22 |
| SIGURG | 23 | SIGXCPU | 24 |
| SIGXFSZ | 25 | SIGVTALRM | 26 |
| SIGPROF | 27 | SIGWINCH, terminal extension | 28 |
| SIGPOLL | 29 | reserved, no legacy name | 30 |
| SIGSYS | 31 | runtime-private cancellation | 32 |
| runtime-private timer | 33 | runtime-private reserve | 34 |
| SIGRTMIN | 35 | SIGRTMAX | 64 |

Application realtime signals occupy 35..64 inclusively. The descriptive bound
is _NSIG=65, with 64 representable positions. Slots 16 and 30 remain unassigned;
32..34 are excluded from application delivery. Do not add obsolete SIGSTKFLT,
SIGPWR, SIGIOT or SIGUNUSED compatibility names to the first profile. Provider
private definitions may map SIGCANCEL to 32 and SIGTIMER to 33, but need an
implemented and authenticated runtime protocol before use. A reserved slot is
not a substitute for implementing that protocol.

Mask admission normalizes a public set rather than rejecting a filled set.
The evaluated provider's sigfillset writes all bits to one. Public mask inputs
therefore ignore positions 16, 30 and 32..34, and attempts to block SIGKILL or
SIGSTOP are silently ignored. SIG_BLOCK, SIG_UNBLOCK and SIG_SETMASK operate
only on the remaining public positions. Runtime-private masking is managed
separately and cannot be changed through application mask calls; in particular,
a filled application mask cannot block the cancellation protocol. Public mask
queries and saved public mask views clear all of those excluded positions.
These rules also apply to sa_mask and the public uc_sigmask view. Direct
delivery to an unassigned or private position still requires its own admission
check; accepting a bit in a sigset object does not admit that operation.

The handler sentinels are typed function-pointer constants: SIG_DFL has target
encoding 0, SIG_IGN 1, SIG_HOLD 2 for the XSI interface, and SIG_ERR -1. They are
distinct from every declarable function address and must never be called. An
eventual runtime must reserve these encodings and use the selected compiler's
function-pointer representation; they are not wire pointers. `sig_atomic_t`
is signed int, 32 bits, and does not replace inter-thread atomic operations.

## Action and event registries

| Name | Value | Name | Value |
| --- | --- | --- | --- |
| SIG_BLOCK | 0 | SIG_UNBLOCK | 1 |
| SIG_SETMASK | 2 | SA_NOCLDSTOP | 0x01 |
| SA_NOCLDWAIT | 0x02 | SA_SIGINFO | 0x04 |
| SA_ONSTACK | 0x08 | SA_RESTART | 0x10 |
| SA_NODEFER | 0x20 | SA_RESETHAND | 0x40 |
| SS_ONSTACK | 0x01 | SS_DISABLE | 0x02 |
| SIGEV_SIGNAL | 0 | SIGEV_NONE | 1 |
| SIGEV_THREAD | 2 | | |

The action flag mask is 0x7f. Unknown action bits are errors. SS_ONSTACK is
returned state, not permission to switch stacks; input accepts SS_DISABLE or
zero. The block operation selector is an enum-like integer, not a bitmask.
SA_SIGINFO selects the three-argument handler member; the two handler members
share storage. No Linux SA_RESTORER field or flag is exported. Trampoline and
return validation remain part of the pending delivery design.

The candidate MINSIGSTKSZ is 8192 bytes and SIGSTKSZ is 32768 bytes for this v3
profile. These are proposed mechanism budgets, not a bound on application
handler stack use. Publish them only after validating complete and nested
signal frames with all admitted CPU state. Future state profiles must provide
truthful stack requirements rather than silently exceeding the fixed floor.

Non-signal-specific si_code values are SI_USER=0, SI_QUEUE=-1, SI_TIMER=-2,
SI_MESGQ=-3 and SI_ASYNCIO=-4. A provider-facing SI_TKILL=-6 denotes authenticated
thread-directed delivery, not a raw application-supplied sender identity.
Positive code values are scoped by si_signo:

| Group | Names in value order, starting at 1 |
| --- | --- |
| SIGILL | ILL_ILLOPC, ILL_ILLOPN, ILL_ILLADR, ILL_ILLTRP, ILL_PRVOPC, ILL_PRVREG, ILL_COPROC, ILL_BADSTK |
| SIGFPE | FPE_INTDIV, FPE_INTOVF, FPE_FLTDIV, FPE_FLTOVF, FPE_FLTUND, FPE_FLTRES, FPE_FLTINV, FPE_FLTSUB |
| SIGSEGV | SEGV_MAPERR, SEGV_ACCERR |
| SIGBUS | BUS_ADRALN, BUS_ADRERR, BUS_OBJERR |
| SIGTRAP | TRAP_BRKPT, TRAP_TRACE |
| SIGCHLD | CLD_EXITED, CLD_KILLED, CLD_DUMPED, CLD_TRAPPED, CLD_STOPPED, CLD_CONTINUED |
| SIGPOLL | POLL_IN, POLL_OUT, POLL_MSG, POLL_ERR, POLL_PRI, POLL_HUP |

For nonzero si_errno, use the owned positive C errno registry. Native negative
status values do not pass through this field.

## Public layouts

All layouts use ordinary LP64 alignment, with no packed structures. The
[compile fixture](../validation/2026-11-04-c-abi-registry/signals.h) gives their
draft C spelling and assertions. Its draft names are not installed API names.

`union sigval` contains int sival_int and void *sival_ptr: size 8, alignment 8.
The pointer is meaningful only in the receiving process under the eventual
interface contract. Copying a pointer across IPC never establishes access.

The candidate siginfo_t is flat and 48 bytes, alignment 8:

| Member | Type | Offset |
| --- | --- | --- |
| si_signo / si_code / si_errno / si_pid | int / int / int / pid_t | 0 / 4 / 8 / 12 |
| si_uid / si_status | uid_t / int | 16 / 20 |
| si_addr | void * | 24 |
| si_band | long | 32 |
| si_value | union sigval | 40 |

These fields are independent, avoiding an oversized legacy compatibility
union. Only fields defined for the given signal/code are meaningful; producers
zero the remainder. The first profile does not expose Linux-only timer,
syscall, protection-key or bounds-fault overlays. Adding public members after
release would require an ABI-version decision, not an unnoticed size change.
For CLD_EXITED, si_status retains the full signed-int exit value. Only the
older wait/waitpid status view extracts its low eight bits; do not reuse that
lossy conversion for waitid or SIGCHLD information.

| Type | Members and offsets | Size / alignment |
| --- | --- | --- |
| struct sigaction | handler union 0, sa_mask 8, sa_flags 16, reserved u32 20 | 24 / 8 |
| stack_t | ss_sp 0, ss_size 8, ss_flags 16, reserved u32 20 | 24 / 8 |
| struct sigevent | sigev_notify 0, sigev_signo 4, sigev_value 8, sigev_notify_function 16, sigev_notify_attributes 24 | 32 / 8 |

The handler union contains void (*sa_handler)(int) and
void (*sa_sigaction)(int, siginfo_t *, void *). The notification callback takes
union sigval by value. Notification attributes point to the provider's
pthread_attr_t; this fixes pointer layout without defining that object's
representation. The reserved words in sigaction and stack_t are ignored on
input and zeroed on output: conforming C callers need to initialize only the
standard members. They do not acquire a private padding-initialization duty.
The versioned context extension below has a separate validation gate. These
are libc application views, not native service records or kernel-private
overlays.

## Context proposal and release gate

Do not expose Kryo's fixed 1024-byte internal save buffer as public ABI. The
candidate mcontext_t is a 216-byte, eight-aligned descriptor with these fields:

| Member | Type | Offset |
| --- | --- | --- |
| version / size | u32 / u32 | 0 / 4 |
| gregs | u64[18] | 8 |
| fs_base / gs_base | u64 / u64 | 152 / 160 |
| xstate | pointer to signal-frame bytes | 168 |
| xstate_bytes / xstate_format | u32 / u32 | 176 / 180 |
| xstate_features | u64 | 184 |
| fault_address / fault_error | u64 / u64 | 192 / 200 |
| trap / reserved | u32 / u32 | 208 / 212 |

Version is 1 and size is 216. Register indices 0..17 are RAX, RBX, RCX, RDX,
RSI, RDI, RBP, RSP, R8, R9, R10, R11, R12, R13, R14, R15, RIP and RFLAGS.
The proposed ucontext_t contains uc_flags at 0, uc_link at 8, uc_sigmask at 16,
uc_stack at 24 and uc_mcontext at 48: size 264, alignment 8. No uc_flags bits
are assigned yet; unsupported bits must not be silently accepted.

The first candidate xstate_format is 1, standard XSAVE encoding with a
64-byte-aligned payload and declared component mask. The v3 execution profile
requires x87, SSE and YMM preservation (mask 7), including the small x87 state
needed by the retained long-double ABI. Additional state cannot appear merely
because the physical processor supports it. The descriptor keeps the payload
length separate from the header and from current kernel allocation size.

This is a reviewable context view, not a chosen signal-return protocol. Before
publishing it, define signal-frame ownership and bounds, nested delivery,
allowed context edits, lifetime under VM retirement, FS/GS and RFLAGS
validation, xstate payload validation and exact restore permissions. Never
execute XRSTOR directly from an unchecked application pointer. A returned
context must not install supervisor addresses, reserved control bits or state
from a newer profile without admission. The payload pointer and uc_link are
process-local and cannot be service wire fields.

## Unimplemented behavior

No signal delivery, sigreturn, alternate-stack execution, process dispositions,
pending queues or inheritance are implemented by this milestone. A future
implementation must review process-versus-thread targeting, signal and IPC
interruption, accepted-operation restart, realtime queue accounting, cancellation
cleanup, default stop/continue/terminate actions and unmaskable SIGKILL/SIGSTOP.
Current fatal native exceptions remain the existing kernel policy; publishing
SIGSEGV or SIGFPE numbers does not make those exceptions recoverable.

The evaluated mlibc pthread cancellation path expects SIGCANCEL, SI_TKILL,
Sigaction and Tgkill behavior and authenticates the sender. A compact mask does
not supply those operations. Unsupported operations must report a defined error
before publishing success; cancellation, thread completion and resource reuse
must preserve Kryo's execution-lease ordering.

## References

- [POSIX.1-2017 signal declarations, licensed republication](https://man7.org/linux/man-pages/man0/signal.h.0p.html)
- [POSIX sigaction contract, licensed republication](https://man7.org/linux/man-pages/man3/sigaction.3p.html)
- [POSIX exit-status views, licensed republication](https://man7.org/linux/man-pages/man3/_Exit.3p.html)
- [Evaluated mlibc generic signal-set implementation](https://github.com/managarm/mlibc/blob/07cfd943977737b8147c1289922c23479feef53d/options/internal/generic/sigset.cpp)
- [Evaluated mlibc pthread signal dependencies](https://github.com/managarm/mlibc/blob/07cfd943977737b8147c1289922c23479feef53d/options/posix/generic/pthread.cpp)

The standards prescribe the named interface requirements; numeric assignments,
compact layouts, reserved runtime positions and context descriptor above are
Cryolite proposals. None establishes POSIX conformance or a built libc.
