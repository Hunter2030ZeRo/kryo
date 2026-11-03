# Candidate Cryolite C-v1 foundation

Status: proposed, not a released ABI or a complete set of libc headers.
See [ADR 0028](../decisions/0028-cryolite-owned-c-abi.md).

## Execution and scalar profile

The first profile targets x86-64-v3, little-endian ELF64 and the SysV AMD64
function-call convention. Native capabilities arrive separately from the
argc/argv/envp/auxv stack. This profile does not provide Linux syscall or binary
compatibility.

| C item | Candidate representation |
| --- | --- |
| CHAR_BIT / plain char | 8 / signed |
| _Bool | size 1, alignment 1 |
| short / int / long / long long | signed 16 / 32 / 64 / 64 bits |
| object pointer | 64 bits, alignment 8 |
| size_t | compiler __SIZE_TYPE__, unsigned long in this profile |
| ptrdiff_t / intptr_t | compiler-derived signed long, 64 bits |
| uintptr_t | compiler-derived unsigned long, 64 bits |
| wchar_t | signed int, 32 bits |
| float / double | binary32 / binary64, size 4 / 8 |
| long double | x87 extended value in 16-byte storage, alignment 16; not binary128 |
| ssize_t / off_t / time_t / clock_t | signed long, 64 bits |
| pid_t / clockid_t | signed int, 32 bits |
| uid_t / gid_t / mode_t / id_t | unsigned int, 32 bits |
| ino_t / dev_t / nlink_t | unsigned 64-bit integers |
| blkcnt_t / blksize_t | signed 64-bit integers |
| fsblkcnt_t / fsfilcnt_t | unsigned 64-bit integers |

The signed public PID type supports the negative selectors required by POSIX
interfaces. Positive PIDs are allocated in a separate runtime/service namespace.
They do not encode rights or reveal native handle slot/generation bits.

Width alone is insufficient for compiler-defined types: using unsigned long long
instead of the compiler's unsigned long size_t changes C++ type identity.
C uses the compiler's wchar_t typedef; C++ keeps its distinct builtin wchar_t
type with the same signed 32-bit representation. The unsigned id_t represents
nonnegative identity values; negative PID selectors remain in signed pid_t.
Do not enable short-enum or structure-packing options for public declarations.
Use fixed-width integers, not C enum layout, in explicit native records.

## Time representation

Candidate timespec layout:

| Field | Type | Offset | Size |
| --- | --- | --- | --- |
| tv_sec | time_t, signed 64-bit | 0 | 8 |
| tv_nsec | long, signed 64-bit | 8 | 8 |

Total size is 16, alignment 8. Normalized nanoseconds are in [0, 1000000000).
Negative wall-time seconds are representable. Function-specific duration rules
still apply; a negative relative timeout is not automatically a valid duration.
Conversions check multiplication, addition, signedness and representable range.

Kryo's monotonic nanoseconds are not Unix epoch time. A future CLOCK_REALTIME
requires an actual wall-time source; CLOCK_PROCESS_CPUTIME_ID and
CLOCK_THREAD_CPUTIME_ID require accounting, not timer ticks or batch wall time.
Numeric clock IDs and seek constants await the owned registry review.

## Errors and identifiers

The existing native result uses RAX=status and RDX=value. Zero status indicates
success; a native failure is a negative status. A successful handle may have its
high bit set. Inspecting RDX's sign is not an error test.

Public errno values will be distinct positive int constants in an owned,
append-only numeric registry. Zero is not an assigned error. Proposed aliases
are EWOULDBLOCK=EAGAIN and EOPNOTSUPP=ENOTSUP. Numeric assignments are deliberately
not frozen in this foundation proposal; no host errno header fills the gap.

Translation requirements:

| Native/runtime condition | POSIX-facing treatment |
| --- | --- |
| Memory backing/metadata exhaustion | ENOMEM |
| Thread creation resource exhaustion | EAGAIN |
| Runtime descriptor table exhausted | EMFILE; distinct from system-wide ENFILE |
| Invalid runtime FD | EBADF; an invalid internal capability is not automatically an invalid user FD |
| Live execution range blocks retirement | EBUSY where the requested operation is otherwise admitted |
| Private park value changed | EAGAIN or an internal retry result |
| Actual wait deadline expired | ETIMEDOUT for interfaces with that contract |
| Unsupported operation | Defined ENOSYS or ENOTSUP according to the interface |
| IPC peer closed | Operation-specific EPIPE, ECONNRESET or EIO; not unconditional EOF |
| Native wait cancelled | Runtime cancellation handling; not automatically a POSIX signal or EINTR |
| Accepted operation later times out/fails | Preserve delivery uncertainty and buffer ownership |

FDs are signed int indices in a runtime/service descriptor table. A capability
handle is opaque u64. A PID, provider mutex-owner ID and pthread_t are separate
identifiers. Do not cast among them. POSIX pthread comparison uses pthread_equal;
the provider's TCB pointer or opaque pthread object is not wire authority.

## Constants, public structures and wire records

The owned registries must name every accepted flag, its mask and invalid
combination rules. Do not forward PROT_*, MAP_*, O_*, clock IDs or ioctl values
directly into native calls. Standard-required permission values such as 0400,
0200 and 0100 remain standard values. Other values are chosen for clear semantics
and compatibility, not deliberately changed merely to differ from another OS.

Before implementation, review at least:

| Area | Remaining decision |
| --- | --- |
| Memory | PROT/MAP values; exact unsupported combinations; partial unmap and protection-change behavior |
| Files | Owned stat layout, timestamps, allocated-block unit, wide inode/device conversion and overflow; open/seek/wait flags |
| Signals | Signal-set capacity and representation, signal numbers, sigaction/siginfo/context layouts and delivery/cleanup rules |
| Resources/network/terminal | The provider build's required public types, constants and complete unsupported-operation policy |
| Provider objects | Versioned pthread object sizes/alignment and compiler/runtime TCB profile; no kernel-private overlays |

BetAFS uses 4096-byte storage blocks; that alone does not define the unit of
public stat.st_blocks. Define this C view and convert from the filesystem's real
allocation accounting. Keep native filesystem IDs and metadata independent of
the POSIX structure; reject unrepresentable results rather than truncate them.

Service wire records use explicit integer widths and encodings with version,
length/opcode and reserved-field rules. Byte order, offsets and interpretation
are protocol choices. Pointer values, compiler padding, C long/size_t and libc
objects do not become a protocol through memcpy. A repr(C) Rust type guarantees
only its declared layout for that target; it does not supply versioning or rights.

## Provider integration gates

At evaluated mlibc commit 07cfd943977737b8147c1289922c23479feef53d:

- The demo port installs fifty ABI headers. The sixteen mandatory sysdep hooks
  are not the complete public-type dependency closure.
- Internal signatures include file, signal and resource types; the loader reads
  stat device/inode fields. An unmodified build waits for those reviewed layouts.
- A structured sigset_t must expose the provider's expected __sig array at
  offset zero, and its size affects accepted signal numbers. A smaller profile
  requires an explicit provider patch and separate review.
- pthread_t is a provider thread-data pointer. Mutex/condition representations
  are provider-owned application ABI, not service payloads.
- Mutex owner IDs fit thirty bits. Supply a unique provider ID mapping instead
  of the fallback value one; keep native thread IDs and counts independent.
- The provider's didExit store precedes its native ThreadExit. Actual Kryo
  completion is required before reclaiming TLS/stack or reusing an owner ID.

The next implementation gate is a reviewed header set and error/flag registry,
not successful stubs or placeholder structures. Provider revision, dependency
revisions, compiler flags and eventual sysroot contents must be reproducible.
