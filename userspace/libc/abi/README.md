# Experimental Cryolite C profile

These source headers implement the reviewed C-v1 candidate definitions under
an explicit `cryolite/` include path. They are not installed system headers, a
stable ABI release, an mlibc `abi-bits` port or a complete libc sysroot. Include
them only when compiling the isolated Cryolite target; mixing them with a host
libc or another provider's type definitions is unsupported.

`types.h` uses the compiler's size, pointer and integer types and checks the
selected little-endian LP64 and 80-bit long-double representations. `time.h`
owns time structures; file and signal structures use ordinary alignment.
The errno header defines numbers, not an errno object or runtime storage.
Declaring a clock, file flag or signal number does not implement that operation.
No syscall, filesystem, signal or POSIX function is declared as working here.
The pthread_attr_t declaration is incomplete and exists only to type the
sigevent pointer. It assigns no object size or representation. The eventual
provider adapter must complete or reconcile that tag before exposing pthread
objects or headers.

The public views remain separate from native Kryo statuses, capabilities and
service encodings. `st_blocks` counts 512-byte public units; BetAFS adapters must
convert actual allocation accounting with overflow checks. Signal masks use
eight bytes, and context descriptors keep the xstate payload separate from the
kernel's private save area. Reserved input fields in ordinary POSIX structures
are ignored; outputs are zero. The experimental versioned context still needs
an explicit validation and lifetime protocol before signal return exists.

The source contract is in [C-v1](../../../docs/abi/c-v1.md), with separate
[error](../../../docs/abi/c-v1-errors.md),
[memory/time](../../../docs/abi/c-v1-memory-time.md),
[file](../../../docs/abi/c-v1-files.md) and
[signal](../../../docs/abi/c-v1-signals.md) definitions. Every native production
compiler path must retain x86-64-v3 and disable MMX generation. The kernel's
soft-float boundary remains independent.

Run `rtk proxy python3 tools/check_c_headers.py --outputdir /tmp/kryo-c-headers`
from the repository root. The checker compiles each header and compares the
combined C/C++ layouts with the earlier candidate fixtures, then checks the
numeric tables and native SDK constants against their source definitions.
Its separate Linux-host wait-status test executes only macros, never native
calls. These checks establish header consistency, not a working libc or POSIX
signal implementation.

## Provider work

At evaluated mlibc revision `07cfd943977737b8147c1289922c23479feef53d`, the
demo port's fifty headers are only a starter inventory. Resource, network,
terminal and directory-entry types, limits and provider pthread objects remain
outside this set. Complete that dependency closure before installing a sysroot
or claiming a provider build. Runtime descriptors, clocks, VM range semantics,
signal delivery, TLS/TCB lifecycle and actual completion before reclamation
still require working native adapters.

The provider's generic signal-set layout accepts this compact mask, but its
signed signal subtraction needs prevalidation for INT_MIN. Its public time
header also exports unselected clock names unconditionally. Internal lock owner
IDs need a distinct, unique thirty-bit mapping. Its allocator asserts successful
backing allocation at this revision; a failing native allocation alone does not
give malloc a correct ENOMEM path. The debug allocator additionally needs
mapping-replacement semantics that Kryo does not yet provide.

A future reproducible provider build must pin all four wrapped dependencies,
preserve the supplied freestanding-header patches, use actual x86 startup and
sysdeps, and link audited target compiler builtins. Host headers, the RISC-V
demo startup, successful stubs and deterministic canaries labeled as entropy
are not substitutes. The provider's explicit legacy string dispatch also needs
review for the v3-only native profile.
