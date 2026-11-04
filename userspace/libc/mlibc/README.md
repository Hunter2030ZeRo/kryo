# Static mlibc bring-up

The Cryolite overlay targets mlibc revision
`07cfd943977737b8147c1289922c23479feef53d`. It selects the ANSI/static profile;
POSIX, Linux, BSD and glibc options are disabled. It does not provide a complete
Cryolite sysroot, pthreads, file descriptors, stdio streams or dynamic linking.

The x86-64 entry preserves Kryo's startup capability from RDI separately from
the SysV argument stack. Startup checks the immutable record, unique roles,
kind-specific rights and actual capability information before choosing optional
role 1 DebugConsole WRITE authority. Logging uses escaped native diagnostics;
these diagnostics do not implement stdout or stderr. Open fails with ENOSYS;
read, write, seek, close and isatty fail with EBADF because this profile installs
no runtime descriptor table.

`__dlapi_enter` performs upstream static loader initialization, ELF/TLS parsing,
TCB setup and constructors. The overlay completes the owned pthread attribute
tag with the pinned provider's real representation. It does not publish thread
creation: native raw threads must not enter this libc until unique lock-owner
IDs, provider TCB creation and completion-aware stack/TLS reclamation exist.

Private mappings accept null hints, anonymous private RW memory and offset zero.
Each mapping owns one reservation containing a private metadata page followed by
its payload. The linked registry admits only exact bases and rounded lengths;
failed release retains the owner. Memory returned after failed commit is released
before an ordinary allocation error; failed rollback stops the process so the
kernel can retire its retained resources. Talc's own backing adapter is separate.

Futex waits use Kryo's private aligned word waits. The provider's relative timeout
is converted once to an absolute monotonic deadline with checked arithmetic.
Realtime and CPU-time clocks fail with ENOTSUP; unselected clocks fail EINVAL.
Unsupported mappings and file operations are never reported as successful.

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
libc headers are used.

Run `rtk proxy python3 tools/boot_smoke.py --profile debug --cpu Haswell-noTSX --libc-probe`
after fetching the archives. `make LIBC_PROBE=1 iso` adds the same optional ELF.
The kernel executes two fresh instances and checks completion and complete
retirement before printing the LIBC success markers. Ordinary boot omits it.
The [decision](../../../docs/decisions/0034-static-mlibc.md) records the patch
scope and remaining production gates.
