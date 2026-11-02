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
AT_PAGESZ, AT_ENTRY and terminating nulls. Incoming RDI separately identifies an
immutable version-1 startup record: service 50, instance 1, argument 0, and one
role-1 WRITE-only debug capability. The probe validates the record and actual
granted handle. It emits event 90/value 2 before exiting successfully.

Native yield and sleep run with live 256-bit patterns in YMM0 and YMM15. An
AVX2 integer loop keeps YMM15 live without calls. Event 90/value 0 marks its
start; kernel validation must observe an increased preemption count before the
value-1 event. Both markers run inside the assembly block with YMM state live.
Value 2 reports success only after storing and checking the final vector.
These checks cover native entry state, not a general
ELF TLS, constructor, exception or language runtime implementation.

Build with `python3 tools/build_abi.py --profile debug|release`. The script builds
core and compiler builtins for the custom target in `build/abi`, using the
installed nightly rust-src and Clang. It packages with the existing ELF entry/
load-data preservation check and records compiler revisions, target/lock/image
hashes and the build command beside the packaged ELF as `.build.json`.

The default build is offline. On a new Rust source installation, add
`--fetch-dependencies` once to permit fetching its locked sysroot dependencies.

ELF is the binary file format; SysV AMD64 defines the function-call convention.
Neither implies Linux syscall compatibility. This probe does not implement libc,
Rust std, libc++, ELF TLS or dynamic linking. Native call version 0 and this
bootstrap target remain experimental. The compiler revisions recorded by the
builder are validation inputs, not an upstream-supported or distributed sysroot.
`targets/cryolite-toolchain.json` pins the validated Rust commit and Clang
version; the builder rejects another revision before compiling this target.
The kernel's rolling toolchain configuration remains independent.
