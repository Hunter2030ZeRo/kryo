# Owned C ABI design checks

Date: 2026-11-03. Drafts: [ADR 0028](../../decisions/0028-cryolite-owned-c-abi.md)
and [candidate foundation](../../abi/c-v1.md).

This milestone contains design and compile-only evidence. It installs no public
C headers, changes no kernel call, builds no libc and executes no new Kryo image.
The preceding native-thread validation remains separate historical evidence.

## Checked representation

The actual pinned Clang 23.1.4 compiler compiled [layout.c](layout.c) as C11 and
C++23 for the existing Cryolite target's LLVM target and x86-64-v3 baseline.
Assertions cover LP64 widths, signed char/PID/wchar representation, compiler
size_t/ptrdiff_t type identity, alignment and floating-point radix/precision/
exponent/storage. The candidate timespec is 16 bytes/alignment 8 with fields at
offsets 0 and 8. A native Buffer example is 16 bytes/alignment 8 with offsets
0, 8 and 12.

The pinned Rust compiler compiled [layout.rs](layout.rs) for the x86-64 Linux
host. It checks the same candidate timespec and imports the actual
interfaces/src/virtio.rs Buffer type for size, alignment and offset checks.
This is a host layout check, not a Cryolite Rust target or runtime execution.
Neither probe is an installed header or a provider implementation.

All three compilation commands completed successfully with warnings denied.
[evidence.json](evidence.json) records their commands, compiler versions,
compiler-pin agreement and input hashes. [layout-checks.log](layout-checks.log)
is the raw compiler output; successful quiet compiles produce an empty log.

## Pinned provider research

Read mlibc commit `07cfd943977737b8147c1289922c23479feef53d` in a separate
research checkout. No provider code was installed in Kryo. The evidence records
hashes for the examined source and wrap files, and the demo port's fifty ABI
headers. That inventory is a starter set, not a demonstrated minimum x86-64
compilation closure.

The pin selects frigg `b9ce1199489f25add06d66c5cc355e2c17dbe027` and libsmarter
`f7d061bc37d485418344452c7ceb28d5df3ba85d`. This milestone does not fetch/build
those libraries or the freestanding header dependencies.

Source inspection confirms public pthread representations remain provider ABI,
the private TCB remains runtime-owned, and a missing FutexTid implementation
falls back to one. Thirty-bit mutex owner IDs require a provider-local mapping;
they do not narrow native capability IDs or kernel thread counts. The provider
sets didExit before invoking its ThreadExit hook, so reclamation still needs
actual native completion.

File, signal and resource headers are included by provider internals, and its
loader uses stat fields. An unmodified build therefore waits for reviewed public
layouts; unsupported sysdep errors alone do not remove those compile-time type
dependencies. No placeholder structure or successful stub closes that gate.

## Review boundary

Standards review used the licensed POSIX.1-2017 text republications linked in
ADR 0028; the original Open Group pages returned HTTP 403 during this check.
The signed PID/offset types, timespec field types, positive errno policy and
permitted aliases are consistent with that text. Permission values explicitly
specified by POSIX must remain unchanged even in an owned ABI.

Public errno numbers, VM/file/clock flags, file/signal layouts and the final
provider object ABI remain review gates. No ABI stability, source compatibility,
binary compatibility, libc adoption or performance result is established here.
