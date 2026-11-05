# Static mlibc bring-up

The Cryolite overlay targets mlibc revision
`07cfd943977737b8147c1289922c23479feef53d`. It selects the ANSI/static profile;
POSIX, Linux, BSD and glibc options are disabled. A native file descriptor
backend now supports ANSI FILE operations through service IPC. A complete
Cryolite sysroot, public POSIX file functions, inherited standard streams,
pthreads and dynamic linking remain pending.

The x86-64 entry preserves Kryo's startup capability from RDI separately from
the SysV argument stack. Startup checks the immutable record, unique roles,
kind-specific rights and actual capability information before choosing optional
role 1 DebugConsole WRITE authority. Logging uses escaped native diagnostics;
these diagnostics do not implement stdout or stderr. An optional role 2 SEND EndpointGate supplies namespace authority. Without it,
Open returns ENOSYS. The fallible descriptor registry reserves metadata before
Open and pins records across operations and close. Descriptors 0..2 remain
uninstalled. Valid regular files return ENOTTY from isatty; absent descriptors
return EBADF. See the [file contract](../../../docs/decisions/0040-service-backed-files.md)
for RPC limits, runtime scratch addresses and accepted-operation ownership.

`__dlapi_enter` performs upstream static loader initialization, ELF/TLS parsing,
TCB setup and constructors. The overlay completes the owned pthread attribute
tag with the pinned provider's real representation. C11 creation, join and detach
now use dedicated provider TCB/static-TLS storage and native completion-aware
retirement. A process-local raw reaper handles detached children. Arbitrary raw
threads with dummy FS storage must not enter provider code. The native allocator
probe and the internal reaper use only their separate native interfaces.

Creator pins protect final publication against fast self-detach. Floating-point
environment, errno and compiler TLS are per thread. The final application thread
exits through provider exit(0), including atexit/global cleanup. The initial
thread has no native self-control grant and cannot be joined or detached through
this profile. POSIX attributes, cancellation and dynamic TLS remain unsupported.

Private mappings accept null hints, anonymous private RW memory and offset zero.
Each mapping owns one reservation containing a private metadata page followed by
its payload. The linked registry admits only exact bases and rounded lengths;
failed release retains the owner. Memory returned after failed commit is released
before an ordinary allocation error; failed rollback stops the process so the
kernel can retire its retained resources. Talc's own backing adapter is separate.

Futex waits use Kryo's private aligned word waits. The provider's relative timeout
is converted once to an absolute monotonic deadline with checked arithmetic.
Realtime and CPU-time clocks fail with ENOTSUP; unselected clocks fail EINVAL.
Unsupported mappings and file features are never reported as successful.

The local stdio storage patch checks FILE allocation before opening a descriptor,
so allocation failure cannot strand an opened owner. Buffer allocation and size
overflow return ENOMEM without null-pointer arithmetic; failed pushback also
sets the stream error indicator. A read publishes consumed pushback as a short
read before further I/O; fread retains complete elements across a later read,
reset or write-back error. The separate files.cpp adapter supplies the descriptor backend.

The optional provider probe exhausts its private memory account, checks public
stdio buffer/pushback failure, and recovers after releasing allocations. Most
pressure uses one native VM reservation with bounded commit sizes; malloc
drains only existing allocator slack. The probe restores errno and the
floating-point environment and leaves stdout without
dirty data. `tests/stdio_fallibility.py` separately compiles the generated target's
verbatim fopen, buffer-allocation, read and fread bodies against host-only
allocator, Open and I/O doubles. Its 25 cases include consumed pushback before
buffered/unbuffered EIO and EOF, complete/incomplete elements, reset/write-back
errors and buffer OOM. They run under AddressSanitizer and
UndefinedBehaviorSanitizer at O0/O2. Source-manifest and function hashes identify
the tested code; successful mock Open is not native filesystem evidence. The separate native
file probe exercises real FILE calls against a volatile Rust validation peer;
it does not mount BetAFS or establish persistent storage.

The file adapter uses indexed descriptor lookup and reuses its vacant mapping
slots. Table and record growth are fallible. A description with an accepted
native IPC failure rejects further I/O, so buffered cleanup cannot replay an
uncertain write. Close remains available; undelivered failures may retry.
`tests/file_runtime.py` runs the verbatim backend and native acceptance helper
against host allocator/native-call doubles, including allocation failures,
close/reuse interleavings and accepted-versus-undelivered cancellation. Its 30
cases run at O0/O2 with ASan/UBSan; this evidence is separate from guest execution.

The build disables stack-protector instrumentation. No AT_RANDOM entropy is
fabricated: this provider's deterministic fallback canary is a bring-up
limitation, not a security guarantee. A real entropy source and enforced hardened
startup remain required before production use.

Header overlays preserve the existing Cryolite scalar, file, signal and memory
proposals. The local resource and scheduling declarations complete static
provider dependencies; no corresponding resource or scheduling service exists.
Their definitions remain experimental. Provider include directories must precede
upstream directories, and these overlays must be the final installed copies.

Build with `rtk proxy python3 tools/build_libc.py --fetch-dependencies` from the
repository root. Downloads are accepted only at the revisions and SHA-256 values
in `sources.json`; subsequent builds work offline. `CRYOLITE_LIBC_CACHE` selects
an existing archive cache. All source extraction, patched copies and sysroot
installation stay under the generated build directory. Source/overlay drift
invalidates reuse; unowned source trees are preserved. No upstream fork or host
libc headers are used. Meson is forced to use the four pinned wrapped dependencies;
compiler/linker environment overrides are removed from the validated build.

Run `rtk proxy python3 tools/boot_smoke.py --profile debug --cpu Haswell-noTSX --libc-probe`
after fetching the archives. `make LIBC_PROBE=1 iso` adds the same optional ELF.
The kernel executes three fresh instances, including final-thread exit and a
single-wait-credit stress case, then checks complete retirement before printing
the LIBC success markers. Ordinary boot omits it.
The [decision](../../../docs/decisions/0034-static-mlibc.md) records the patch
scope; the [thread contract](../../../docs/decisions/0035-provider-thread-lifetime.md)
records publication, detach and lifetime rules. Internal thread-runtime hooks
belong to the pinned provider adapter, not application interfaces.
