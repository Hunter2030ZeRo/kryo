# Candidate C-v1 file ABI

Status: proposed. These definitions extend the [foundation](c-v1.md); they do
not install headers, implement a VFS or release a stable ABI. Numeric choices
belong to Cryolite. Familiar values are retained where useful, without importing
Linux headers, Linux syscalls or ICE behavior.

## File metadata

The proposed `struct stat` has size 128 and alignment 8. Use normal LP64
alignment, not packing. Types come from the foundation; every reserved field
must be zero on output. No field contains a capability or physical address.

| Field | Type | Offset | Size |
| --- | --- | --- | --- |
| st_dev | dev_t, unsigned long | 0 | 8 |
| st_ino | ino_t, unsigned long | 8 | 8 |
| st_nlink | nlink_t, unsigned long | 16 | 8 |
| st_mode | mode_t, unsigned int | 24 | 4 |
| st_uid | uid_t, unsigned int | 28 | 4 |
| st_gid | gid_t, unsigned int | 32 | 4 |
| __reserved0 | unsigned int | 36 | 4 |
| st_rdev | dev_t, unsigned long | 40 | 8 |
| st_size | off_t, signed long | 48 | 8 |
| st_blksize | blksize_t, signed long | 56 | 8 |
| st_blocks | blkcnt_t, signed long | 64 | 8 |
| st_atim | struct timespec | 72 | 16 |
| st_mtim | struct timespec | 88 | 16 |
| st_ctim | struct timespec | 104 | 16 |
| __reserved1 | unsigned long | 120 | 8 |

`st_dev` and `st_ino` together identify an object in the VFS namespace; hard
links name the same identity. A service must provide collision-free 64-bit
identities or a maintained translation, not truncate or hash a wider filesystem
identifier and assume uniqueness. Public UIDs/GIDs are descriptive identities;
filesystem access still requires actual service authority and permission checks.

`st_size` counts logical bytes. `st_blksize` is a positive I/O-size hint, not
an allocation unit or a minimum transfer size. The candidate `st_blocks` unit
is **512 bytes**, chosen for familiar application accounting; POSIX does not
mandate this unit. Convert the filesystem's actual per-object allocation count.
For a count expressed in 4096-byte BetAFS blocks this requires checked
multiplication by eight and a signed-64-bit range check. Do not derive allocation
from rounded logical length; sparse files, metadata and shared allocations need
the filesystem's explicit accounting policy.

The timestamp fields contain Epoch-based seconds and normalized nanoseconds,
not Kryo monotonic time. `st_ctim` is metadata-change time, not creation time.
The eventual public header supplies `st_atime`, `st_mtime` and `st_ctime` aliases
to the corresponding `tv_sec` fields. Missing filesystem support is an explicit
integration gate; a port must not invent metadata to obtain a successful stat.
Reject an unrepresentable result with `EOVERFLOW` before publishing a partial
output structure.

### Mode registry

These constants have type `int` and are valid preprocessing expressions. The
permission and special-mode values retain POSIX's specified octal values.
File-type encodings are Cryolite choices using established values.

| Constant | Value |
| --- | --- |
| S_IFMT | 0170000 |
| S_IFIFO | 0010000 |
| S_IFCHR | 0020000 |
| S_IFDIR | 0040000 |
| S_IFBLK | 0060000 |
| S_IFREG | 0100000 |
| S_IFLNK | 0120000 |
| S_IFSOCK | 0140000 |
| S_ISUID | 04000 |
| S_ISGID | 02000 |
| S_ISVTX | 01000 |
| S_IRWXU | 0700 |
| S_IRUSR | 0400 |
| S_IWUSR | 0200 |
| S_IXUSR | 0100 |
| S_IRWXG | 0070 |
| S_IRGRP | 0040 |
| S_IWGRP | 0020 |
| S_IXGRP | 0010 |
| S_IRWXO | 0007 |
| S_IROTH | 0004 |
| S_IWOTH | 0002 |
| S_IXOTH | 0001 |
| UTIME_NOW | 0x3fffffff |
| UTIME_OMIT | 0x3ffffffe |

The `S_IS*` tests compare `(mode & S_IFMT)` with one file-type value. Creation
and chmod modes accept only `07777`; they do not accept file-type bits. This
does not authorize set-ID execution: credentials, capability transfer and
executable transition policy need separate implementation and review. Special
`tv_nsec` sentinels are interpreted only by timestamp-setting APIs; they are
never normalized timestamp outputs.

## Open and descriptor flags

All open constants have type `int`. Access modes are named values selected by
`O_ACCMODE`; they are not independently combinable flags. The only admitted
access-mode values are 0, 1, 2, 4 and 8. In particular, 3 is invalid rather than
an alias for execute-only. The remaining entries are independent bits.

| Constant | Value |
| --- | --- |
| O_RDONLY | 0x00000 |
| O_WRONLY | 0x00001 |
| O_RDWR | 0x00002 |
| O_EXEC | 0x00004 |
| O_SEARCH | 0x00008 |
| O_ACCMODE | 0x0000f |
| O_CREAT | 0x00010 |
| O_EXCL | 0x00020 |
| O_TRUNC | 0x00040 |
| O_APPEND | 0x00080 |
| O_NONBLOCK | 0x00100 |
| O_DIRECTORY | 0x00200 |
| O_NOFOLLOW | 0x00400 |
| O_CLOEXEC | 0x00800 |
| O_NOCTTY | 0x01000 |
| O_DSYNC | 0x02000 |
| O_RSYNC | 0x04000 |
| O_SYNC | 0x08000 |
| O_TTY_INIT | 0x10000 |
| FD_CLOEXEC | 0x00001 |

The known open-bit mask is `0x1ffff`. Reject unknown bits or invalid access
modes with `EINVAL`. For behavior POSIX leaves undefined, this profile rejects
`O_EXCL` without `O_CREAT`, and `O_TRUNC` without `O_WRONLY` or `O_RDWR`.
`O_CREAT|O_DIRECTORY` is rejected with `EINVAL`; directory creation uses mkdir.
`O_DIRECTORY` and `O_SEARCH` require a directory; `O_EXEC` requires a
non-directory. A valid recognized feature without its required implementation
returns the interface's unsupported-operation error; it must not lose the flag
and report success.

Creation/lookup flags are `0x11e70`; open-description status flags are `0x0e180`.
`F_GETFL` returns the access mode and status flags, not creation/lookup flags.
`F_SETFL` changes only `O_APPEND|O_NONBLOCK` (`0x180`); known immutable bits
for access and creation are ignored. Until synchronization-mode changes are
implemented, a request changing the `0xe000` synchronization bits fails with
`ENOTSUP` before any flag changes; unchanged bits from `F_GETFL` remain valid.
Unknown bits remain invalid. Supporting synchronization-mode changes is a
separate POSIX-completion gate, not an ignored-success implementation.
`FD_CLOEXEC` belongs to a descriptor entry; duplicate descriptors share open
description offset/status, not descriptor flags. No operation forwards these
values directly to native capabilities or service opcodes.

An append write needs service-side offset selection and writing as one
operation. `O_SYNC` dominates `O_DSYNC`; with `O_RSYNC`, reads require the
selected data/file integrity guarantee too. An IPC reply or buffer copy alone
does not prove durable completion. Close-on-exec duplication/creation must
publish the flag atomically with the descriptor. Terminal flags require real
terminal policy; they are not an excuse to initialize a terminal in the kernel.

### Descriptor control and lock records

| Constant | Value |
| --- | --- |
| F_DUPFD | 0 |
| F_GETFD | 1 |
| F_SETFD | 2 |
| F_GETFL | 3 |
| F_SETFL | 4 |
| F_GETLK | 5 |
| F_SETLK | 6 |
| F_SETLKW | 7 |
| F_DUPFD_CLOEXEC | 8 |
| F_GETOWN | 9 |
| F_SETOWN | 10 |
| F_RDLCK | 0 |
| F_WRLCK | 1 |
| F_UNLCK | 2 |
| SEEK_SET | 0 |
| SEEK_CUR | 1 |
| SEEK_END | 2 |

Descriptor-control commands and lock types are separate registries. Unknown
commands are `EINVAL`; only `FD_CLOEXEC` is valid in `F_SETFD`. Duplication
returns the lowest available nonnegative descriptor at or above its checked
minimum. Signal-owner commands do not expose native task IDs or confer authority
to signal another process. Locking and signal-owner operations remain pending.

The proposed `struct flock` is 32 bytes, alignment 8:

| Field | Type | Offset | Size |
| --- | --- | --- | --- |
| l_type | short | 0 | 2 |
| l_whence | short | 2 | 2 |
| __reserved0 | unsigned int | 4 | 4 |
| l_start | off_t, signed long | 8 | 8 |
| l_len | off_t, signed long | 16 | 8 |
| l_pid | pid_t, signed int | 24 | 4 |
| __reserved1 | unsigned int | 28 | 4 |

Reserved input fields are ignored; reserved output fields are zeroed. A caller
need only initialize the standard input fields. A zero length represents an
interval extending to the end of the file. Positive and
negative lengths use checked signed offset/range arithmetic; absolute starts
must not precede zero. The locking service owns process-scoped lock lifetime,
conflict checks and atomic wait registration; ordinary Kryo word waits do not
implement file locks. A blocking request resolves its byte range before it
waits; later file-size or shared-offset changes must not move that request.
Checked seek arithmetic rejects negative final offsets
with `EINVAL` and unrepresentable offsets with `EOVERFLOW`.

### Relative lookup, access and advice

| Constant | Value |
| --- | --- |
| AT_FDCWD | -100 |
| AT_EACCESS | 0x01 |
| AT_SYMLINK_NOFOLLOW | 0x02 |
| AT_SYMLINK_FOLLOW | 0x04 |
| AT_REMOVEDIR | 0x08 |
| F_OK | 0 |
| X_OK | 1 |
| W_OK | 2 |
| R_OK | 4 |
| POSIX_FADV_NORMAL | 0 |
| POSIX_FADV_RANDOM | 1 |
| POSIX_FADV_SEQUENTIAL | 2 |
| POSIX_FADV_WILLNEED | 3 |
| POSIX_FADV_DONTNEED | 4 |
| POSIX_FADV_NOREUSE | 5 |

`AT_FDCWD` is a runtime namespace selector, not a valid native handle. Each
function admits only its own `AT_*` mask: faccessat accepts `0x03`, fstatat,
fchmodat, fchownat and utimensat accept `0x02`, linkat accepts `0x04`, and
unlinkat accepts `0x08`. Other combinations are `EINVAL`. Access checks accept
only the `0x07` permission mask; an existence check grants no open authority.
Advice changes cache policy rather than correctness or durability. No advice
operation is implemented by the present descriptor-free native runtime.

## Poll representation

The proposed `nfds_t` is unsigned long (64 bits). `struct pollfd` has size 8,
alignment 4: signed int `fd` at 0, signed short `events` at 4 and signed short
`revents` at 6.

| Constant | Value |
| --- | --- |
| POLLIN | 0x001 |
| POLLPRI | 0x002 |
| POLLOUT | 0x004 |
| POLLERR | 0x008 |
| POLLHUP | 0x010 |
| POLLNVAL | 0x020 |
| POLLRDNORM | 0x040 |
| POLLRDBAND | 0x080 |
| POLLWRNORM | 0x004 |
| POLLWRBAND | 0x100 |

The input request mask is `0x1c7`; error/hangup/invalid bits are output-only
and are ignored in `events`. Unknown input bits outside `0x1ff` are invalid.
A negative `fd` is ignored and gets zero `revents`; an invalid nonnegative
descriptor contributes `POLLNVAL`, not a fabricated ready notification.
Input/output readiness requires real service state and an atomic subscribe/
recheck contract. A timeout of zero checks without blocking, any negative
timeout waits without a timeout, and a positive value uses a checked monotonic
millisecond deadline. Regular files report read/write readiness without a
storage-completion wait. A count beyond the process descriptor limit is
`EINVAL`; temporary allocation failure for poll bookkeeping is `EAGAIN`.
Neither result permits losing live subscriptions or borrowed buffers.

## Process wait view

These constants describe a future POSIX process view; native join is a distinct
capability operation. Process parentage, process groups, stop/continue delivery,
reaping and cancellation have not been implemented by these definitions.

| Constant | Value |
| --- | --- |
| WNOHANG | 0x01 |
| WUNTRACED | 0x02 |
| WSTOPPED | 0x02 |
| WCONTINUED | 0x04 |
| WEXITED | 0x08 |
| WNOWAIT | 0x10 |
| P_ALL | 0 |
| P_PID | 1 |
| P_PGID | 2 |

The proposed `idtype_t` is a C enum with those three values, size/alignment 4
under this compiler profile; it is never a service-wire enum. `waitpid` admits
mask `0x07`; `waitid` admits `0x1f` and requires at least one of `WEXITED`,
`WSTOPPED` or `WCONTINUED`. A `waitpid` PID of -1 selects any child, zero
selects the caller's process group, and other negative values select the
checked absolute process-group ID. Taking the absolute value of `INT_MIN`
must not overflow; that selector has no representable group in this profile
and therefore selects no child (`ECHILD`).

The candidate 32-bit wait-status word preserves a familiar encoding: normal
exit is `(code & 255) << 8`; signal termination stores signal 1..126 in bits
0..6, with optional core-produced bit 7; a stop is `(signal << 8) | 0x7f`;
continued is `0xffff`. No bit above 15 is emitted. `WIFEXITED` tests zero low
byte, `WIFSTOPPED` tests low byte `0x7f`, `WIFCONTINUED` tests `0xffff`, and
`WIFSIGNALED` tests a nonzero low-seven-bit value other than `0x7f`.
`WEXITSTATUS`/`WSTOPSIG` extract bits 8..15; `WTERMSIG` extracts bits 0..6.
Extraction is meaningful only after its matching predicate. The signal profile
currently proposes 64 signal numbers; raising it above 126 would require a
different status version. Native fault and exit words are translated deliberately,
never reinterpreted as this integer. The core bit is set only after an actual
core artifact exists; defining a bit does not implement a core-dump facility.

waitid returns the full signed-int exit value in si_status for CLD_EXITED;
its result is not this packed wait-status word. With WNOHANG and no matching
status, it returns zero with si_pid and si_signo zero. Do not leave those fields
uninitialized or fabricate a child completion.

## Provider and validation boundary

At evaluated mlibc commit `07cfd943977737b8147c1289922c23479feef53d`, the
loader reads `st_dev` and `st_ino`. Generic fdopen obtains `F_GETFL` and adds
`O_APPEND` through `F_SETFL`; this requires preserving the other known flags.
No generic `O_EXEC`, `O_SEARCH` or `O_ACCMODE` value assumption was found in
the inspected options sources. A commented dirent check calls out `O_PATH` as
Linux-specific; this profile does not define it. This targeted source audit
does not prove a complete provider build or dependency closure.

[Compile fixtures](../validation/2026-11-04-c-abi-registry/files.h) spell out
the candidate layouts and constants without host headers. C/C++ layout checks
do not establish executable VFS, storage durability, poll or POSIX wait support.
Resource, directory-entry, terminal and network structures remain separate
provider-integration work; they are not inferred from this document.

## References

- [POSIX fcntl declarations, licensed republication](https://man7.org/linux/man-pages/man0/fcntl.h.0p.html)
- [POSIX fcntl behavior, licensed republication](https://www.man7.org/linux/man-pages/man3/fcntl.3p.html)
- [POSIX open behavior, licensed republication](https://man7.org/linux/man-pages/man3/open.3p.html)
- [POSIX stat fields and permission values, licensed republication](https://man7.org/linux/man-pages/man0/sys_stat.h.0p.html)
- [POSIX poll representation, licensed republication](https://man7.org/linux/man-pages/man0/poll.h.0p.html)
- [POSIX poll behavior, licensed republication](https://www.man7.org/linux/man-pages/man3/poll.3p.html)
- [POSIX wait declarations, licensed republication](https://man7.org/linux/man-pages/man0/sys_wait.h.0p.html)
- [POSIX wait behavior, licensed republication](https://man7.org/linux/man-pages/man3/wait.3p.html)
- [POSIX waitid behavior, licensed republication](https://man7.org/linux/man-pages/man3/waitid.3p.html)
- [Evaluated mlibc source](https://github.com/managarm/mlibc/tree/07cfd943977737b8147c1289922c23479feef53d)
