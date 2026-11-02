# Native ELF and user ABI

Date: 2026-11-02. Status: format supported; general user ABI pending.

## Executable format

Native Kryo/Cryolite images use little-endian ELF64 for AMD64. The current loader
accepts static ET_EXEC, EM_X86_64, EI_OSABI=0, ABI version zero and e_flags=0.
It loads PT_LOAD bytes, zeroes BSS and enforces RO/RW/RX permissions and W^X.
NULL, NOTE, PHDR and non-executable GNU_STACK descriptions are accepted within
their supported bounds. Unknown extensions, PT_INTERP, PT_DYNAMIC and PT_TLS
are rejected. PIE, shared objects, dynamic linking and ELF TLS are not implemented.

The bootstrap wiring bounds images to 64 program headers, eight load segments,
16 load pages and one MiB of module bytes. These are current loader policy limits,
not ELF format limits. Packaging verifies entry and load bytes remain identical
after stripping debug sections. The userspace linker emits elf64-x86-64 images
with separate executable, read-only and writable load segments at 0x400000.

## ABI boundaries

ELF format does not by itself specify all process and operating-system behavior.
The native system-call ABI, startup capabilities, runtime services, dynamic
linker and thread lifecycle belong to Cryolite. Linux ELF execution remains an
ICE-Linux compatibility path with Linux ABI semantics. PE execution belongs to
the independent ICE-NT path.

Use System V AMD64 function-call conventions for the future general native user
toolchain, with a separately tested C-compatible FFI boundary. Keep the kernel's
freestanding target and FP/SIMD restrictions independent from user compilation.
Preserving XMM state is required but does not configure a compiler's calling ABI.

## Current bootstrap limitations

Existing Rust boot programs still use x86_64-unknown-none with small/static code
model overrides. The installed target specifies rustc-abi=softfloat and disables
SSE/SSE2. They do not establish general hard-float SysV AMD64 C/C++ interoperability.
No dedicated Cryolite target or standard-library sysroot is validated here.

Manifest startup currently passes a read-only startup-record handle in RDI;
legacy startup passes two handles in RDI/RSI. The initial stack is aligned but
does not contain argc/argv/envp/auxv. A private TLS page and FS setup do not
implement PT_TLS templates or language TLS destructors. There is no general crt0,
constructor invocation, dynamic linker or exception/unwind runtime yet.

## Next executable gate

Build a dedicated x86-64-v3 hard-float user target and validate real C-to-Rust and
Rust-to-C mixed integer/floating calls across native entries. Then add the
defined process initial stack and crt0. Do not fabricate AT_PHDR unless program
headers are actually mapped, or AT_RANDOM without real supplied entropy. The
current loader's executable mappings do not include its ELF header by default.

The platform minimum is x86-64-v3, including OS-enabled AVX/YMM state; SSE2-only
compilation is not the final native target. Early hardware admission is now
separate from conservative kernel code generation. See
[platform requirements](0022-platform-baseline-and-performance.md).

Only after these foundations and the common native services are tested should
the target/sysroot claim libc, libc++ or Rust std support. BetAFS remains a normal
Ring 3 std consumer. See [runtime direction](0019-native-runtime-and-standard-libraries.md).

References: [ELF gABI](https://gabi.xinuos.com/elf/00-foreword.html) separates object
format from processor and execution contracts; [x86-64 psABI](https://gitlab.com/x86-psABIs/x86-64-ABI)
defines the machine-specific ABI.
