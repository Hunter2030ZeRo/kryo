# Candidate C-v1 error registry

Date: 2026-11-04. Status: proposed; no installed headers or released numbers.
This registry extends the [C-v1 foundation](c-v1.md). It defines the public
libc vocabulary, not Kryo native statuses or service protocol errors.

## Representation and allocation

Each public error macro expands to a positive `int` constant usable in `#if`.
Zero remains the success value and is not an error assignment. The first table
assigns the POSIX.1-2017 header inventory in alphabetical order, excluding the
two aliases below. This makes the initial registry compact and straightforward
to audit; no numerical relationship to a native status is intended. After a
release, additions append unused numbers and never reorder this table, reuse
withdrawn assignments or insert values to restore alphabetical order.

The inventory reserves names independently of subsystem implementation.
STREAMS-related and standard-reserved names do not request STREAMS, a
compatibility kernel or other legacy services. New implementation-specific
errors require a documented application contract before receiving a number.
The pinned provider's optional `EIEIO` string case does not require exporting
that provider-specific sentinel; C-v1 does not assign it.

| Symbol | Value |
| --- | --- |
| E2BIG | 1 |
| EACCES | 2 |
| EADDRINUSE | 3 |
| EADDRNOTAVAIL | 4 |
| EAFNOSUPPORT | 5 |
| EAGAIN | 6 |
| EALREADY | 7 |
| EBADF | 8 |
| EBADMSG | 9 |
| EBUSY | 10 |
| ECANCELED | 11 |
| ECHILD | 12 |
| ECONNABORTED | 13 |
| ECONNREFUSED | 14 |
| ECONNRESET | 15 |
| EDEADLK | 16 |
| EDESTADDRREQ | 17 |
| EDOM | 18 |
| EDQUOT | 19 |
| EEXIST | 20 |
| EFAULT | 21 |
| EFBIG | 22 |
| EHOSTUNREACH | 23 |
| EIDRM | 24 |
| EILSEQ | 25 |
| EINPROGRESS | 26 |
| EINTR | 27 |
| EINVAL | 28 |
| EIO | 29 |
| EISCONN | 30 |
| EISDIR | 31 |
| ELOOP | 32 |
| EMFILE | 33 |
| EMLINK | 34 |
| EMSGSIZE | 35 |
| EMULTIHOP | 36 |
| ENAMETOOLONG | 37 |
| ENETDOWN | 38 |
| ENETRESET | 39 |
| ENETUNREACH | 40 |
| ENFILE | 41 |
| ENOBUFS | 42 |
| ENODATA | 43 |
| ENODEV | 44 |
| ENOENT | 45 |
| ENOEXEC | 46 |
| ENOLCK | 47 |
| ENOLINK | 48 |
| ENOMEM | 49 |
| ENOMSG | 50 |
| ENOPROTOOPT | 51 |
| ENOSPC | 52 |
| ENOSR | 53 |
| ENOSTR | 54 |
| ENOSYS | 55 |
| ENOTCONN | 56 |
| ENOTDIR | 57 |
| ENOTEMPTY | 58 |
| ENOTRECOVERABLE | 59 |
| ENOTSOCK | 60 |
| ENOTSUP | 61 |
| ENOTTY | 62 |
| ENXIO | 63 |
| EOVERFLOW | 64 |
| EOWNERDEAD | 65 |
| EPERM | 66 |
| EPIPE | 67 |
| EPROTO | 68 |
| EPROTONOSUPPORT | 69 |
| EPROTOTYPE | 70 |
| ERANGE | 71 |
| EROFS | 72 |
| ESPIPE | 73 |
| ESRCH | 74 |
| ESTALE | 75 |
| ETIME | 76 |
| ETIMEDOUT | 77 |
| ETXTBSY | 78 |
| EXDEV | 79 |

| Alias | Value |
| --- | --- |
| EWOULDBLOCK | EAGAIN |
| EOPNOTSUPP | ENOTSUP |

Both alias pairs are permitted by the referenced standard. `EEXIST` and
`ENOTEMPTY` remain distinct. No additional historical aliases are selected.
The public list therefore has 81 names and 79 distinct positive values.

## Translation and errno storage

Decode native `RAX` status independently of `RDX` value. Successful values may
have their high bit set. An unknown or malformed backend result is an internal
protocol failure, never a new public error number obtained by negation or cast.

The runtime translates at the operation boundary. A global status-to-errno
cast loses information about authority, delivery and resource ownership.

| Condition and operation | Candidate public treatment |
| --- | --- |
| Native call absent | ENOSYS |
| Recognized optional operation absent | ENOTSUP where that operation permits it |
| Invalid public flags, range or scalar | EINVAL before side effects |
| Invalid caller address for a required buffer | EFAULT where safely detectable |
| Memory allocation or VM backing/metadata quota exhausted | ENOMEM |
| Thread admission resources exhausted | EAGAIN |
| Runtime descriptor slots exhausted | EMFILE; system-wide file resources use ENFILE |
| Closed or invalid public file descriptor | EBADF |
| Descriptor not open for the requested read/write operation | EBADF; path permission checks use EACCES |
| Native capability permission failure | Operation-specific EACCES or EPERM; never inferred from a descriptive FD number |
| Live execution lease blocks an otherwise admitted mapping change | EBUSY |
| Private park comparison finds a different value | Internal retry or EAGAIN for an interface exposing that result |
| A contractually timed wait reaches its deadline | ETIMEDOUT |
| Stat or other result cannot fit the public representation | EOVERFLOW; no truncation |
| Filesystem storage allocation exhausted | ENOSPC; EDQUOT only for a defined filesystem quota condition |
| Peer closes a byte stream | EOF only after the service confirms orderly end of input; EPIPE, ECONNRESET or EIO as the operation requires |
| Native wait cancelled | Runtime cancellation handling; not automatically EINTR or POSIX cancellation |
| Unknown backend status or malformed service reply | EIO plus internal diagnostics; do not publish a guessed error mapping |

`EBUSY` does not override existing native validation precedence. Legacy
MemoryMap/MemoryUnmap rejects the reserved VM arena before execution-lease
checks; a rejected public request must not be retried through another mechanism
merely to obtain a preferred error number.

Error reporting does not revoke accepted IPC delivery, shared-buffer loans,
execution leases or DMA references. The runtime retains the owners and delivery
state until actual cancellation/completion and retirement. A timeout cannot
authorize replay of an operation whose execution is uncertain.

`errno` belongs to compiler TLS. Only public interfaces whose contract reports
failure through `errno` write it. Successful calls need not clear it. Interfaces
such as pthread functions, `clock_nanosleep` and `posix_madvise` return their
positive error directly as specified; sysdep hooks return an error to libc
without writing `errno` themselves. Internal park retries do not force a TLS
write. This keeps error translation off successful and noncontended paths.

## Review and verification gates

- Check all 79 primary assignments for positivity, uniqueness and stable order;
  check both alias pairs and preprocessing usability in generated header tests.
- Verify operation-specific translation with injected failures and ensure
  successful high-bit native values survive unchanged.
- Verify per-thread errno isolation only after an actual libc/TCB integration.
  A scalar or constant compile check does not establish TLS behavior.
- Audit the selected provider's active code against this explicit inventory.
  Unsupported extensions must be feature-gated or translated explicitly;
  another port's errno header is not a fallback.

## References

- [POSIX.1-2017 errno header, licensed republication](https://man7.org/linux/man-pages/man0/errno.h.0p.html)
- [Pinned provider's error string implementation](https://github.com/managarm/mlibc/blob/07cfd943977737b8147c1289922c23479feef53d/options/ansi/generic/string.cpp)
- [Native result encoding](../interfaces.md)
