# Shared file descriptions and descriptor control

Date: 2026-11-05. Base: `5303b3197dd282fb89993569a92d469fd6e03cc3`.

The static provider now duplicates and atomically replaces descriptors while
retaining one shared open-description owner. Gate authority, request sequence,
I/O lock, status and uncertain state are shared; CLOEXEC stays descriptor-local.
These are user-runtime mechanisms over the existing native file protocol. No
kernel syscall, wire value or public C ABI layout was changed.

## Results

| Scope | Result |
| --- | --- |
| Verbatim file backend with host native-call doubles | 46 cases at O0/O2, ASan/UBSan |
| Pinned stdio function bodies with host doubles | 25 cases at O0/O2, ASan/UBSan |
| Python tool/parser checks | 38 passed |
| C/C++ header/compiler checks | 43 passed |
| Actual provider builds | Debug and release passed |
| Normal/all-feature kernel builds, format and Clippy | Passed; Clippy warnings denied |
| QEMU integrated matrix | Four final configurations passed |

The four configurations are debug/Haswell-noTSX/one CPU,
release/max/four CPUs with read-only block storage,
debug normal boot/max/four CPUs with writable block storage, and
release/Haswell-noTSX/one CPU. Each runs ABI and libc probes; every libc probe
runs three fresh process instances. The boundary release case uses an isolated
copy with separate Cargo, provider and ISO outputs. Packaged images, generated
backend source and all eight probe inputs are hash-checked. Existing thread,
TLS, SIMD, allocation-failure, file, capability and applicable block checks
remain required. The normal configuration reaches the persistent supervisor.

## Executed behavior

The C application uses the real private provider hooks against the separate
Rust file-service peer. It checks shared position, source close followed by
alias I/O, F_DUPFD minimum selection, descriptor-local CLOEXEC, filtered status,
same-number rules, atomic replacement of a different open description, rejected
status changes and table-growth quota failure preserving existing descriptors.
Explicit duplication populates initially absent descriptors 0 and 1. A temporary
file-backed stdout receives fputs/fflush, and an alias reads the exact bytes
back. The probe closes its descriptors without closing the global FILE object.
This does not install inherited terminal streams or a console service.

Trusted LIBCFD success prints only after all three application runs and their
file peers exit with complete object/wait/channel/account/heap/frame retirement.
The earlier file probe still leaves one native descriptor for actual process
retirement. Duplicates retain local description references rather than minting
new service gates, so the peer sees final closure once per open description.

Host cutpoints separately cover in-flight replacement, numerical reuse,
publication close, shared poison after accepted cancellation, mutable local
flags, table failure, and unpublished Open reservations. Those deterministic
interleavings are not native simultaneous descriptor-I/O evidence. Two negative
controls remove alias ownership retention or put descriptor flags in shared
status; both fail the corresponding regressions. ASan/UBSan are enabled;
LeakSanitizer is disabled under the host constraint, while explicit owners,
tables, mappings and memory-capability counters assert retirement.

## Corrections retained

The initial provider build exposed that Dup2/Fcntl tags are not declared with
the POSIX option off. Their tag adapters are now conditionally compiled; private
hooks remain implemented and exercised in the ANSI profile. POSIX as a whole
was not enabled to make the build pass.

The first guest reached stage856 and rejected a test assertion that fflush on
an absent descriptor1 must succeed. The corrected probe checks EBADF before
binding, then performs unchanged write/flush/readback checks after duplication.
The failed log and metadata are retained and excluded from passing results.

## Rust std direction and limits

[ADR 0041](../../decisions/0041-cryolite-rust-std.md) records the accepted Unix
public surface with Cryolite-specific platform implementations and owned ABI
bindings. rust-std-inventory.json records 16 actual pinned Rust module hashes,
the compiler revision and current target cfg. Generic Unix startup/pthread
paths cannot be enabled by merely adding a target family. No installed compiler
or Rust source was patched, no std sysroot was built, and std=false remains.
The current descriptor work is a prerequisite, not proof of Rust std support.

The provider stays ANSI/static. Fcntl/Dup2 tag bridges exist only when upstream
POSIX tags are declared; private hooks provide the tested current surface.
Actual append/nonblocking/synchronization changes remain ENOTSUP, while
unchanged status and ignored immutable bits are accepted. Exec, descriptor
inheritance, public POSIX functions, readiness, PTY, production VFS, mounted
BetAFS and complete standard libraries remain pending.

See [the ownership contract](../../decisions/0042-descriptor-duplication.md).
Actual-source verification records the executed commands and hashes; results.json
binds all final guest and archived metadata to the source manifest. Unchanged
core tests are not recounted. Documentation-only finalization changes no
executable input. Host/native results do not establish hardware performance.
