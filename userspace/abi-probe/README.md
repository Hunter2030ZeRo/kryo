# Native ABI probe

This isolated freestanding ELF64 executable links independently compiled Clang C,
Clang C++ and Rust objects using the Cryolite user target. It tests mixed
integer/double calls in both directions, the ninth floating argument on the
stack, C variadic floating arguments and mixed integer/double aggregate returns.
It does not link a C or C++ standard library.

The user target selects x86-64-v3 with the SysV AMD64 hard-float convention. It is
separate from the kernel's soft-float `x86_64-unknown-none` target. Existing native
bootstrap programs retain their current target while this ABI is validated.

Entry captures the SysV initial RSP before making a C-convention CALL into Rust.
The fixture requires argv `abi-probe`, `cryolite-v3`, environment `KRYO_ABI=1`,
AT_PHDR, AT_PHENT, AT_PHNUM, AT_PAGESZ, AT_ENTRY and terminating nulls. The probe
reads the actual mapped ELF header through `__ehdr_start` and checks those
auxiliary values against its read-only header/program-header load. Incoming RDI separately identifies an
immutable version-1 startup record: service 50, instance 1, argument 0, and one
role-1 WRITE-only debug capability. The probe validates the record and actual
granted handle. It emits event 90/value 2 before exiting successfully.

Native yield and sleep run with live 256-bit patterns in YMM0 and YMM15. An
AVX2 integer loop keeps YMM15 live without calls. Event 90/value 0 marks its
start; kernel validation must observe an increased preemption count before the
value-1 event. Both markers run inside the assembly block with YMM state live.
Value 2 reports success only after storing and checking the final vector.
These checks cover native entry state, not a general constructor, exception or
language runtime implementation.

The dedicated linker maps the ELF header and program headers read-only, keeps
the TLS initialization template in a read-only load, and emits PT_TLS for native
C `__thread`, C++ POD `thread_local` and Rust `#[thread_local]` objects. These
objects use local-exec TLS with initialized data, zero-filled storage and
64-byte alignment.

The runtime fixture allocates three fixed private BSS slots: one for the main
thread and one for each of two native child threads. It copies PT_TLS initialized
bytes, clears the remaining storage and puts a minimal self-pointer TCB after
the aligned Variant II block before setting FS. Only zero-residue aligned TLS
templates are supported by this fixture. The TCB is sufficient for these
local-exec accesses; it is not mlibc's complete TCB, DTV, canary or cancellation
layout. This is not a general allocation interface.

Each language checks initial data, zero-fill, alignment and private values
across yield/sleep. Children join before the main thread clears and checks their
slots; the main thread's values must remain unchanged. Event 94/value 1 marks
main TLS initialization, and value 2 marks completed joins, slot retirement and
isolation checks. The call-free AVX2 preemption interval runs afterward with no
child runnable. POD TLS needs no destructor callbacks; dynamic TLS and TLS
destructors remain unimplemented.

Build with `python3 tools/build_abi.py --profile debug|release`. The script builds
core and compiler builtins for the custom target in `build/abi`, using the
installed nightly rust-src and Clang. It packages with the existing ELF entry/
load-data preservation check and records compiler revisions, target/lock/image
hashes and the build command beside the packaged ELF as `.build.json`.

The default build is offline. On a new Rust source installation, add
`--fetch-dependencies` once to permit fetching its locked sysroot dependencies.

ELF is the binary file format; SysV AMD64 defines the function-call convention.
Neither implies Linux syscall compatibility. This probe does not implement libc,
Rust std, libc++ or dynamic linking. Native call version 0 and this
bootstrap target remain experimental. The compiler revisions recorded by the
builder are validation inputs, not an upstream-supported or distributed sysroot.
`targets/cryolite-toolchain.json` pins the validated Rust commit and Clang
version; the builder rejects another revision before compiling this target.
The kernel's rolling toolchain configuration remains independent.
