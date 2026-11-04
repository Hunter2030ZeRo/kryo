# Candidate C-v1 memory and clock registries

Date: 2026-11-04. Status: proposed; constants do not establish implementations.
See the [foundation](c-v1.md) and [error registry](c-v1-errors.md).
These values belong to the libc view. Translate them into native calls instead
of forwarding the flag integer into a kernel or service operation.

## Memory constants

The common low-bit protection layout and mapping-kind values keep validation
simple and avoid changing familiar values without a semantic reason. They are
explicit Cryolite definitions, not an imported host ABI header.

| Symbol | Value |
| --- | --- |
| PROT_NONE | 0x00 |
| PROT_READ | 0x01 |
| PROT_WRITE | 0x02 |
| PROT_EXEC | 0x04 |
| MAP_SHARED | 0x01 |
| MAP_PRIVATE | 0x02 |
| MAP_FIXED | 0x10 |
| MAP_ANONYMOUS | 0x20 |
| MAP_ANON | MAP_ANONYMOUS |
| MS_ASYNC | 0x01 |
| MS_INVALIDATE | 0x02 |
| MS_SYNC | 0x04 |
| MCL_CURRENT | 0x01 |
| MCL_FUTURE | 0x02 |
| POSIX_MADV_NORMAL | 0 |
| POSIX_MADV_RANDOM | 1 |
| POSIX_MADV_SEQUENTIAL | 2 |
| POSIX_MADV_WILLNEED | 3 |
| POSIX_MADV_DONTNEED | 4 |

`MAP_FAILED` is the target pointer value `(void *)-1`, not a native negative
status or a valid user mapping. No `MAP_FILE` flag is needed: absence of the
anonymous bit selects the descriptor-backed form. Huge-page, grow-down,
populate, no-reserve, locked-map and Linux-specific advice flags are not selected
by this profile. Defining the POSIX advisory/locking vocabulary does not enable
those optional operations.

Validation masks are `(prot & ~0x07) == 0` and `(flags & ~0x33) == 0`. Exactly one
of `MAP_SHARED` and `MAP_PRIVATE` is required. A caller must not combine
`MS_ASYNC` and `MS_SYNC`; the synchronization mask is `0x07`. The locking mask
is `0x03`, with at least one bit for `mlockall`. Advice values are a selector,
not an OR-able mask. Unknown bits and malformed combinations return `EINVAL`
before changing mappings. A recognized unsupported mode returns the operation's
documented failure, never success with discarded flags.

## First native adapter scope

The existing kernel supplies reserve/commit/decommit/release, not a POSIX VM
implementation. The first provider-private anonymous allocator can use one
reservation per allocation, commit zeroed RW/NX backing before success, and
release the exact recorded reservation on free. It validates the original and
rounded lengths with checked arithmetic. A failed commit must release its
reservation or retain an explicit cleanup owner. Metadata allocation failure
must leave no usable-but-untracked mapping. Physical continuity is not required.

This allocator primitive must not silently become a complete public `mmap`.

| Public request | Required behavior or remaining mechanism |
| --- | --- |
| Anonymous private RW, no fixed address | Candidate first subset: reserve plus eager commit; zeroed RAM before success |
| Anonymous private PROT_NONE | Reserve only; later protection changes still require their own implementation |
| Anonymous private read-only or write-only | Admit only once requested protection can be enforced; do not publish RW for read-only |
| W+X mapping | Reject under the existing W^X policy |
| Executable mapping or mprotect to executable | Requires immutable code preparation and alias checks; no RW-to-RX success stub |
| Nonfixed nonzero address | A placement hint; ignoring it may be permitted, replacing an occupied mapping is not |
| MAP_FIXED | Requires exact placement, conflicting-range retirement and rollback contract; unsupported initially |
| Descriptor-backed or MAP_SHARED mapping | Requires file/shared-object lifetime, offsets and a service/pager contract; unsupported initially |
| Partial munmap | Requires splitting or releasing reservation ownership for the selected page range; decommit alone retains the range and is insufficient |
| Whole mapping munmap | Retire accessible backing, waits and reservation credit before success; preserve owners on failure |
| mprotect | Requires permission changes and acknowledged TLB retirement; returning success for an unchanged mapping is invalid |
| msync, mlock/mlockall, POSIX advice | Implement defined effects or return the interface's documented unsupported error |

For a selected anonymous extension, require `fd == -1` and `offset == 0`.
`len == 0`, address/length overflow and invalid alignment are errors. Public
lengths need not be page multiples: the adapter rounds outward without wrapping
and tracks the rounded extent. File offsets must be nonnegative and page-aligned
for the initial file-mapping profile. A successful nonfixed mapping is nonzero
and cannot replace existing mappings. Protection and range validation is
independent of whether the requested operation has been implemented.

The whole-reservation release mechanism cannot satisfy general `munmap` across
partial mappings, holes or multiple reservations. Initial allocator-only hooks
may enforce their private exact-allocation contract, but a public POSIX claim
waits for the required range semantics. A live execution lease can return
`EBUSY`; it cannot be bypassed by releasing metadata or forgetting a mapping.
Decommit and release cancel registered private word waits before actual backing
retirement. Errors after incomplete rollback must preserve the current native
quarantine and resource-retention behavior.

The first libc allocator should obtain arenas and manage small allocations in
user space. A page operation or IPC per `malloc` is not the target design.
Address reservations use interval metadata, and explicit eager commit retains
the accepted zeroing and RAM-availability contract. Demand paging, overcommit,
physical contiguity and huge pages are not implied by a public flag definition.

## Clock constants and types

| Symbol | Value |
| --- | --- |
| CLOCK_REALTIME | 0 |
| CLOCK_MONOTONIC | 1 |
| CLOCK_PROCESS_CPUTIME_ID | 2 |
| CLOCK_THREAD_CPUTIME_ID | 3 |
| TIMER_ABSTIME | 0x01 |
| TIME_UTC | 1 |

The first four macros have a representation assignment-compatible with signed
32-bit `clockid_t`; they are selectors, not native handles. `TIMER_ABSTIME` is
the only selected sleep/timer flag. `CLOCKS_PER_SEC` is `((clock_t)1000000)`,
with signed 64-bit `clock_t`; its unit does not promise a one-microsecond hardware
timer or scheduler resolution. `timer_t` is a runtime-owned opaque pointer,
size/alignment 8/8 in this profile, compatible with the evaluated provider's
public type. It does not encode a native timer capability or raw kernel address.

Candidate time structures use the foundation's signed 64-bit `time_t` and long.
`suseconds_t` is signed long, size/alignment 8/8. No packed structures are used.

| Structure | Field | Type | Offset | Size |
| --- | --- | --- | --- | --- |
| timespec | tv_sec | time_t | 0 | 8 |
| timespec | tv_nsec | long | 8 | 8 |
| timeval | tv_sec | time_t | 0 | 8 |
| timeval | tv_usec | suseconds_t | 8 | 8 |
| itimerspec | it_interval | struct timespec | 0 | 16 |
| itimerspec | it_value | struct timespec | 16 | 16 |
| itimerval | it_interval | struct timeval | 0 | 16 |
| itimerval | it_value | struct timeval | 16 | 16 |

`timespec` and `timeval` are 16 bytes; `itimerspec` and `itimerval` are 32 bytes.
All have alignment 8. Normalized nanoseconds are in `[0, 1000000000)` and
normalized microseconds in `[0, 1000000)`. Negative epoch seconds are
representable; they are not automatically valid relative durations.

## Time behavior and provider gates

- Kryo's monotonic nanoseconds can back `CLOCK_MONOTONIC`. The epoch is
  unspecified and the value is nondecreasing. Do not label it wall time or
  promise behavior across suspend until the platform clock contract covers it.
- `CLOCK_REALTIME` and `TIME_UTC` require an actual epoch source. Neither an
  all-zero successful timestamp nor uptime relabeled as Unix time is valid.
- Process/thread CPU clocks require execution accounting. Scheduler slices,
  timer interrupt counts and elapsed SMP batch duration are not substitutes.
- Normalize and check arithmetic before converting seconds to native unsigned
  nanoseconds. Reject a native-deadline overflow instead of wrapping or
  saturating it into a different wait. A relative wait snapshots monotonic time
  and adds its duration with checked arithmetic; an already elapsed admitted
  absolute deadline or zero relative duration completes immediately. Never
  forward a native reserved deadline value as a valid public instant. The
  selected clock's range and the
  public function's error contract govern negative/out-of-range timestamps.
- Only an actual signal interruption produces `EINTR`. Native cancellation
  does not fabricate signal delivery. `clock_nanosleep` returns an error number
  directly; `nanosleep` uses `-1` and `errno`. Absolute sleep does not write a
  remaining-duration output. Relative interruption requires a nonnegative
  remaining interval if that output was requested.
- Unknown clock selectors/flags fail validation. A recognized clock lacking a
  particular operation returns that function's documented unsupported error;
  `clock_nanosleep` rejects its calling-thread CPU clock. Until wall and CPU
  sources exist, their declarations remain vocabulary rather than support.
- The evaluated provider hardcodes these four IDs and additional Linux-oriented
  clock names in its public `time.h`. Integration must explicitly feature-gate
  unselected names or define their unsupported policy; they do not enter this
  registry through provider headers. Timer object creation, intervals, signal
  delivery and lifecycle remain separate implementation gates.

## References

- [POSIX.1-2017 memory header, licensed republication](https://man7.org/linux/man-pages/man0/sys_mman.h.0p.html)
- [POSIX mapping semantics, licensed republication](https://man7.org/linux/man-pages/man3/mmap.3p.html)
- [POSIX range unmapping, licensed republication](https://man7.org/linux/man-pages/man3/munmap.3p.html)
- [POSIX time header, licensed republication](https://man7.org/linux/man-pages/man0/time.h.0p.html)
- [POSIX clock sleep, licensed republication](https://man7.org/linux/man-pages/man3/clock_nanosleep.3p.html)
- [Pinned provider public clock definitions](https://github.com/managarm/mlibc/blob/07cfd943977737b8147c1289922c23479feef53d/options/ansi/include/time.h)
- [Pinned provider timer representation](https://github.com/managarm/mlibc/blob/07cfd943977737b8147c1289922c23479feef53d/options/posix/include/bits/posix/timer_t.h)
