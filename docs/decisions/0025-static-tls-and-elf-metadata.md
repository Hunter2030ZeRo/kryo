# ELF metadata and static TLS

Date: 2026-11-03. Status: standard static TLS direction accepted; bootstrap implemented.

## Ownership

Use SysV AMD64 Variant II static TLS. The user runtime owns template copying,
zero-fill, TCB layout, allocation and destruction. Kryo owns ordinary mappings,
execution resources and per-thread FS-base preservation. It does not interpret
mlibc TCB fields or DTV entries. Dynamic TLS, general pthread cleanup and a full
mlibc TCB remain outside this bootstrap.

The evaluation reference is mlibc commit
`07cfd943977737b8147c1289922c23479feef53d`. Its static startup reads real ELF and
program headers through `__ehdr_start`, and installs early/full TCBs in user
space. This source reference is not a built or adopted Cryolite sysroot.

## Loader contract

Core exposes an ELF header address and program-header address/offset/count/size
only when the corresponding file bytes have unique read-only LOAD backing.
Native startup requires the header plus e_phoff to reach the actual mapped PHDR
array. It supplies truthful AT_PHDR, AT_PHENT and AT_PHNUM with AT_PAGESZ and
AT_ENTRY. No random bytes or auxiliary pointers are fabricated.

PT_PHDR must describe the actual parsed table, precede LOAD descriptions and
match its real read-only mapping. PT_TLS is unique, read-only, range/alignment
checked and has filesz no greater than memsz. Initialized bytes must have exactly
one matching LOAD translation. Zero-only TLS needs no file template. The TLS
zero-fill extent describes per-thread storage and need not be ordinary image
LOAD memory. Dynamic segments remain rejected; existing page/segment limits and
W^X checks remain in force.

The dedicated ABI linker maps ELF/PHDR metadata read-only at 0x400000 and starts
code at 0x401000. Its static TLS template is read-only. Earlier boot programs
retain their shared linker and entry conventions.

## Packaging

Mapping FILEHDR makes ELF section-table fields part of LOAD bytes. Removing
debug sections legitimately changes e_shoff, e_shentsize, e_shnum and e_shstrndx.
The ABI packaging check explicitly normalizes only those ELF64 fields while
comparing entry, program headers and every other loaded byte exactly. The output
itself retains objcopy's valid section metadata. Earlier boot-image packaging
keeps its exact byte comparison. Unit tests reject changes in runtime headers,
program headers and payload under the new comparison.

## Runtime fixture

The opt-in probe uses native compiler local-exec TLS in C, C++ and Rust. The
parent prepares initialized data, zero-filled BSS and TCBs before publishing its
two native children. All three threads check alignment and mutate independent
values across yield/sleep/join. Runtime-owned private BSS slots provide storage;
minimal TCBs contain a self pointer only.
Those TCBs are not mlibc TCBs and implement no DTV or destructor runtime.

The fixture accepts TLS templates with p_vaddr modulo p_align equal to zero and
rejects other layouts. Generic ELF parsing retains valid nonzero residues. A
general runtime must preserve that first-byte offset when placing TLS; simply
rounding memsz works only for the aligned case. Do not claim arbitrary ELF TLS
support from this fixture.

Native completion is observed before clearing a child slot; a user-space
didExit flag alone is insufficient. The TLS phase retires three execution
owners; the kernel verifies final charges and frame ownership after all probe
phases complete. Later private-wait checks add threads independently. Fixture
thread budgets are not global scheduler limits. The fixture uses growing
production registries instead of the old two-thread constructor.

The two compiler-TLS children now use
[ThreadCreateNative](0027-runtime-owned-thread-memory.md) with runtime-owned
stacks and prepared TLS, without legacy scratch allocations. Eight later
private-word children retain legacy ThreadCreate to test its managed TLS
retirement. The probe checks runtime trampoline rounding inheritance while the
kernel supplies fresh CPU data state; this is not complete POSIX fenv or
production allocator support.

## Next contracts

mlibc still needs its complete TCB/DTV/runtime, a user allocator over
[explicit private VM](0026-private-vm-and-atomic-waits.md), wrappers for native
atomic private waits and service-backed streams/files. Its generic exit path can publish
didExit before native ThreadExit; storage reclamation must wait for actual
execution ownership to end. Native capability and lifetime rules remain in
force through timeout, cancellation and thread/process termination.

References:

- [ELF program headers](https://gabi.xinuos.com/elf/07-pheader.html)
- [mlibc static startup](https://github.com/managarm/mlibc/blob/07cfd943977737b8147c1289922c23479feef53d/options/rtld/generic/main.cpp)
- [mlibc TCB](https://github.com/managarm/mlibc/blob/07cfd943977737b8147c1289922c23479feef53d/options/internal/include/mlibc/tcb.hpp)
- [mlibc thread completion](https://github.com/managarm/mlibc/blob/07cfd943977737b8147c1289922c23479feef53d/options/internal/generic/threads.cpp)
- [TLS alignment and placement](https://www.akkadia.org/drepper/tls.pdf)
