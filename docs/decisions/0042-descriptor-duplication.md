# Descriptor duplication and control

Date: 2026-11-05. Status: implemented experimental provider backend.

## Owners

A runtime open description owns one native file gate, immutable access/status
flags, its next request ID, the I/O lock, reusable loan address and uncertain
I/O state. Descriptor entries hold references to that owner and their own
FD_CLOEXEC flag. Operation pins retain the description after close or atomic
replacement. All aliases therefore use the same server cursor and local poison
state, including after an accepted failure with unknown execution.

Duplicating a descriptor within one process adds a local reference. It does not
mint another service gate, allocate a receiver thread or send an Open request.
Final local descriptor/operation retirement closes the native gate; the existing
kernel closure contract then notifies the service. Cross-process inheritance
and transfer remain separate contracts.

Indexed descriptor lookup remains bounded by the table size check. Table growth
is fallible and happens before publication. No fallible operation follows
unpublishing a replacement target. Source identity is retained under the table
lock, the replacement commits atomically, and the old description retires after
the lock is released. Existing operations on the old target keep their own pin.

## Supported control

F_DUPFD returns the lowest free descriptor at or above its nonnegative minimum
with CLOEXEC cleared. F_DUPFD_CLOEXEC publishes the same duplication with CLOEXEC
set. F_GETFD and F_SETFD read/change only descriptor flags. F_GETFL returns access
mode plus open-description status, excluding creation/lookup and CLOEXEC bits.

The Dup2 provider hook atomically replaces an explicit destination. A valid
same-number dup2 is a no-op and preserves descriptor flags. O_CLOEXEC selects
the dup3-style case, rejects equal source/destination and publishes CLOEXEC with
the replacement. Invalid source or allocation failure leaves the destination
unchanged. A destination reserved by an unpublished Open returns EBUSY
without stealing that transaction's slot. None of this implements an exec or
process-inheritance path.

Open still starts allocation at descriptor 3 in the current limited profile.
Descriptors 0..2 are initially absent, but explicit duplication can populate
them, including file-backed stdout redirection. This does not install inherited
terminal streams, PTYs or a console transport.

Status flags remain immutable in the current service protocol. F_SETFL ignores
known access/creation bits and accepts an unchanged status; changing append,
nonblocking or synchronization mode returns ENOTSUP before any mutation.
Unknown bits are EINVAL. Lock and signal-owner commands remain unsupported;
unknown commands are EINVAL. Existing Cryolite numeric values stay unchanged.

## Evidence boundary

The provider remains ANSI/static; POSIX as a whole is disabled. Its Fcntl/Dup2
tag adapters are selected only when the provider declares those POSIX tags.
The private runtime hooks are built and tested in the current profile without
importing host libc layouts or exporting the entire public POSIX file API.

Host source-body tests cover table growth failure, replacement, closed-number
reuse, in-flight pins, descriptor-local flags and shared uncertain state.
Native C checks use the existing Rust file-service peer in another address
space and require actual final closure and resource retirement. Neither scope
establishes a mounted BetAFS service or Rust std support.

The already owned [C file ABI](../abi/c-v1-files.md) defines these values and
ownership rules. The semantic references are the licensed POSIX republications
for [dup/dup2](https://man7.org/linux/man-pages/man3/dup.3p.html) and
[fcntl](https://man7.org/linux/man-pages/man3/fcntl.3p.html). The provider's limited
surface and unsupported operations remain explicit above.
