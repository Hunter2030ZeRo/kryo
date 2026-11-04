# Native runtime and standard libraries

Date: 2026-11-02. Status: implementation direction; runtime ports pending.

## Decision

Cryolite needs a native execution environment shared by C, C++ and Rust. Kryo
and its hardware mechanisms remain `no_std`; Ring 3 services may use their
language's standard library once its Cryolite port is available. BetAFS using
Rust `std` is consistent with this boundary.

Build one native platform contract, then port established libraries onto it.
C needs libc and process startup; C++ additionally needs its ABI runtime,
standard library and the selected exception/unwind support; Rust needs a
Cryolite target and working `std` platform implementations. These are separate
deliverables. A successful freestanding build or custom target JSON does not
establish standard-library support.

Native libraries use Kryo capabilities and Cryolite services. ICE-Linux and
ICE-NT remain separate ABI translation paths; native libc must not silently
depend on either compatibility layer.

## Shared platform contract

Keep kernel calls and service records independent of language layout. Use
explicit integer widths, bounded lengths, opaque handles and versioned records.
Bulk I/O names shared-memory objects and checked offsets, not process pointers.
C headers and Rust wrappers must describe the same representation. Rust enums,
`Result`, `std::io::Error`, C++ objects and exceptions do not cross IPC as native
memory layouts.

| Area | Required native behavior |
| --- | --- |
| Startup | Static ELF entry, stack alignment, owned argv/envp storage, auxiliary information, startup capabilities and a defined `crt0` handoff |
| Memory | Private heap backing, alignment, growth, release and ordinary allocation failure; language allocators stay in user space |
| TLS | ELF TLS templates, per-thread initialization, ABI layout, thread-local errors and destructor order |
| Threads and waits | Creation/join, synchronization and timed blocking with an atomic compare-before-park operation or equivalent lost-wakeup-safe protocol |
| Time | Monotonic deadlines distinct from wall time; no fabricated UTC or extension of a deadline on retry |
| Files and processes | VFS-backed open/read/write/seek/stat/close, descriptor duplication and inheritance, process startup/status, explicit partial-I/O and cancellation results |
| Terminal and networking | PTY/terminal operations and readiness, then socket services with defined blocking and closure semantics |
| Runtime support | Compiler builtins, initialization/finalization, entropy, exception/unwind policy and dynamic linking when required |

File descriptors are a process-runtime interface over capabilities, not numeric
kernel handles cast to `int`. Shared open-file state, offsets, duplication and
close semantics need explicit ownership. Translate native errors into each
library's error model; unsupported operations return an error rather than
simulated success. An accepted timeout may leave I/O uncertain and does not
release a device's buffer ownership.

Current native calls already provide capability IPC, shared-memory objects,
monotonic time, threads, explicit FS-base setup, atomic private word waits and
reserve/commit/decommit/release for private RW/NX RAM. The optional native bootstrap checks a SysV argument stack
and mapped ELF metadata with compiler static TLS. Legacy startup conventions
remain supported. The [static mlibc profile](0034-static-mlibc.md) now supplies
the initial provider TCB, static TLS and a fallible Talc heap. The
[C11 thread extension](0035-provider-thread-lifetime.md) adds completion-aware
provider lifecycle; POSIX attributes/cancellation and service-backed files/streams
are still required. The managed
scratch TLS page and eight-page
shared-object API remain separate mechanisms. See
[interfaces](../interfaces.md) and [the private-VM decision](0026-private-vm-and-atomic-waits.md).

[ThreadCreateNative](0027-runtime-owned-thread-memory.md) now borrows runtime
stack/TLS spans and protects unmapped guards until actual execution completion.
The runtime prepares its TCB before publication and can reclaim backing after
native join. POSIX attributes, complete environment/signal inheritance,
joinable/detached policy and cleanup remain libc/runtime responsibilities.

## Bring-up order

1. Define the native user ABI and toolchain sysroot separately from the kernel
   target. Begin with static x86-64 ELF, a C-compatible call boundary and a
   consistent user FP/SIMD ABI. Preserve the kernel's separate compilation flags.
   Pin compiler/runtime revisions; the final target name and libc provider remain
   undecided. The finite [bootstrap gate](0024-native-bootstrap-stack.md) now
   uses an experimental v3 target and a mixed C/C++/Rust probe. mlibc is the
   primary port candidate; the general runtime/sysroot remains pending.
2. Extend the freestanding startup, compiler TLS and private-VM bootstrap into
   the runtime's `crt0`, full TCB, user allocator and shared platform client.
   Preserve argument, thread-local, allocation-failure and retirement checks in
   actual user execution. Connect the implemented private compare-before-park
   protocol and runtime-owned thread creation; notifications alone do not supply
   atomic private waiting.
3. Port one libc through a small native OS interface. Connect standard streams,
   filesystem descriptors, timing and thread synchronization to real services.
   Add the POSIX-facing surface required by applications incrementally; no
   `fork` requirement is imposed on Kryo merely because a library exposes it.
4. Port Rust `std` platform modules onto the same services, reusing libc where
   its semantics fit and native hooks where capability operations require them.
   Add target support to dependent build crates as needed. An in-tree target
   description, platform implementation and tested sysroot are distinct from
   upstream target acceptance or official binary distribution.
5. Bring up one C++ library/runtime combination. LLVM libc++ with libc++abi is
   a candidate, not a current selection; libstdc++ is an alternative tied to its
   toolchain requirements. Validate constructors, local-static guards, allocation,
   TLS, threads and exceptions. A bootstrap profile with exceptions disabled or
   Rust panic-abort must be labeled as limited. Full support requires the chosen
   unwinder and loader behavior; do not unwind through an incompatible FFI frame.

Dynamic linking, locale/timezone data, signals and broader compatibility follow
the first static programs. PTY and event readiness are required before claiming
an ordinary terminal; the C++ runtime alone does not establish KDE readiness.

## BetAFS integration

Keep the independent BetAFS engine and its normal `std` use. Run it as a Ring 3
filesystem service through the Cryolite Rust target, with the required libc/FFI
support for native codec dependencies. A `no_std` engine rewrite is not the
primary integration path. Only its Kryo block/VFS adapter belongs here.

That adapter must preserve 4096-byte blocks, read-your-writes and durable flush.
Failed commits retain their uncertain state and require reopening under the
engine contract. The runtime port does not replace real block I/O, nor does a
host-file backend prove native execution. See the
[adapter contract](../../userspace/filesystems/betafs/README.md).

## Evidence required

Record independent native C, C++ and Rust programs exercising allocation, TLS,
threads, files and errors. Test unsupported operations and service loss alongside
success. A codec link test, static image load or host standard-library test is
not proof that the corresponding native runtime works.

No libc, C++ standard library or Cryolite Rust `std` port is implemented by this
decision. Current progress remains in [status](../status.md).

## References

- [Newlib OS interface requirements](https://sourceware.org/newlib/libc.html#Syscalls) describe the boundary a C library needs from its host; this is not a libc selection.
- [LLVM runtime vendor documentation](https://libcxx.llvm.org/VendorDocumentation.html) separates libc++, its ABI library and unwind runtime builds.
- [Rust target bring-up](https://rustc-dev-guide.rust-lang.org/building/new-target.html) and [target policy](https://doc.rust-lang.org/rustc/target-tier-policy.html) distinguish target descriptions, dependent crates and standard-library support.
- [Rust standard-library platform modules](https://github.com/rust-lang/rust/blob/master/library/std/src/sys/mod.rs) include allocation, files, synchronization, threads and time in addition to the platform abstraction layer.
