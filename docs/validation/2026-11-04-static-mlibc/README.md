# Static mlibc validation

Date: 2026-11-04. Source base: 49aa79c.

Four final QEMU TCG cases pass:

| Profile | CPU | CPUs | Boot | Additional probes |
| --- | --- | --- | --- | --- |
| Debug | Haswell-noTSX | 1 | Boot tests | Mixed C/C++/Rust ABI |
| Release | max | 4 | Boot tests | Mixed ABI, readonly block service |
| Debug | max | 4 | Normal | Writable block service |
| Release | Haswell-noTSX | 1 | Boot tests | Static libc |

Each guest runs two fresh static mlibc processes. Required C checks cover SysV
arguments/environment, constructors, initialized/zero compiler TLS, errno,
zeroed and aligned allocations, real private-memory quota exhaustion,
failure-preserving realloc, integer/floating snprintf and truncation, monotonic
sleep lasting at least one millisecond, rejected provider thread creation and
files, and atexit/destructors. AVX2 strlen/strcmp operate at a committed page's
last byte with the next page inaccessible. Volatile function pointers prevent
the compiler from substituting the calls under test.

Each process additionally creates two raw native allocator threads. Across 16
rounds they retain concurrent live sets, verify aligned/reallocated contents,
join actual completion, then close controls and release execution backing.
They never enter provider malloc, errno, TCB or lock paths. This is allocator
concurrency evidence, not pthread support or CPU-affinity evidence.

The kernel requires the main thread's distinct completion events, exit 42,
exactly two created children and three retired threads per run. All objects,
waits and account credits retire; logical heap usage and physical ownership
balance independently. Trusted LIBC markers print only after both processes
finish. USERDIAG text cannot satisfy those markers.

Actual checkout verification matches 1,512 source/document inputs, runs 463
Rust tests, 32 Python tests and 32 C header checks, and passes formatting,
normal/all-feature kernel builds, kernel/allocator Clippy with warnings denied,
and both provider builds. A generated-source rename is detected and repaired
from pinned inputs. Provider command audits find only target headers with v3
and MMX-disabled compilation. Linked probes contain no MMX or legacy string
fallback; public AVX2 strcmp may inline its upstream implementation.

The matrix uses an isolated source snapshot. Actual-checkout rebuilds use the
same reviewed inputs and separate generated directories. This is functional
host/emulator evidence, not a bare-metal timing or power result. No complete
hosted libc, descriptor/terminal service, pthreads, entropy/hardened startup,
C++ standard library or Rust std is claimed.

The first source build exposed missing implementation-only ANSI declarations;
the local patches now admit them without enabling public Linux/glibc options.
An earlier code-generation assertion wrongly required a private strcmp symbol
even when its AVX2 body was inlined. It was corrected to inspect the public
function. Those failed-build transcripts remain in the working evidence
directory; none are counted as a passing guest. The five archive pins and six
local patches are committed with the port.

See evidence.json, the source manifest, actual verification record, provider
command audit and the four guest logs/build records beside this file.
