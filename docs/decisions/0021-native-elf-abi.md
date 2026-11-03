# Native ELF and user ABI

Date: 2026-11-03. Status: static format and TLS metadata supported; general runtime pending.

## Executable format

Native Kryo/Cryolite images use little-endian ELF64 for AMD64. The current loader
accepts static ET_EXEC, EM_X86_64, EI_OSABI=0, ABI version zero and e_flags=0.
It loads PT_LOAD bytes, zeroes BSS and enforces RO/RW/RX permissions and W^X.
NULL, NOTE, PHDR and non-executable GNU_STACK descriptions are accepted within
their supported bounds. PT_TLS now describes a validated static template and
per-thread zero-fill extent. Unknown extensions, PT_INTERP and PT_DYNAMIC remain
rejected. PIE, shared objects, dynamic linking and dynamic TLS are not implemented.

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
A separate experimental x86-64-v3 hard-float Cryolite target now serves a mixed
C/C++/Rust freestanding probe. It does not migrate those older programs or
provide a standard-library sysroot. See [bootstrap ABI](0024-native-bootstrap-stack.md).

Manifest startup currently passes a read-only startup-record handle in RDI;
legacy startup passes two handles in RDI/RSI. The initial stack is aligned but
does not contain argc/argv/envp/auxv. The explicit new probe entry uses an owned
SysV-style stack plus a separate startup capability. A private TLS page and FS setup do not
implement PT_TLS templates or language TLS destructors. There is no general crt0,
constructor invocation, dynamic linker or exception/unwind runtime yet.

## Executable gate and remaining runtime

The dedicated x86-64-v3 hard-float probe tests mixed calls across C, C++ and Rust,
owned argument/environment storage, auxiliary values and live AVX2 state.
The current opt-in probe additionally maps real ELF/PHDR metadata and initializes
compiler local-exec TLS in user space. General crt0 and a complete libc runtime
remain pending. Do not fabricate AT_PHDR unless program
headers are actually mapped, or AT_RANDOM without real supplied entropy. The
earlier boot images do not include their ELF header in LOAD mappings; the
dedicated native linker now provides a read-only header mapping.

The platform minimum is x86-64-v3, including OS-enabled AVX/YMM state; SSE2-only
compilation is not the native target. Early hardware admission is
separate from conservative kernel code generation. See
[platform requirements](0022-platform-baseline-and-performance.md).

Only after these foundations and the common native services are tested should
the target/sysroot claim libc, libc++ or Rust std support. BetAFS remains a normal
Ring 3 std consumer. See [runtime direction](0019-native-runtime-and-standard-libraries.md).

References: [ELF gABI](https://gabi.xinuos.com/elf/00-foreword.html) separates object
format from processor and execution contracts; [x86-64 psABI](https://gitlab.com/x86-psABIs/x86-64-ABI)
defines the machine-specific ABI.
