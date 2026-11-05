# Cryolite Rust standard library

Date: 2026-11-05. Status: platform direction accepted; the first restricted
implementation is recorded in [ADR 0043](0043-restricted-native-rust-std.md).

## Platform classification

Keep target_os=cryolite and expose the Unix API family when a working native
platform backend is available. The public std::os::unix surface should allow
established filesystem and runtime code to compile. Its implementation uses
Cryolite's owned C ABI, capabilities and services; it does not select Linux
syscalls, ICE or Linux ABI layouts.

Unix classification is a public API choice. It is not permission to enable the
generic Unix startup and pthread implementations before their contracts exist.
Keep the current freestanding bootstrap target and std=false metadata unchanged
until the corresponding executable probe works. Do not advertise full POSIX
support from target cfg or a successful core/alloc build.

## Implementation boundary

The pinned Rust source dispatches allocation, errors, files, randomness, stdio,
threads, TLS and synchronization independently. A new sys/pal directory alone
does not select all of them. Add Cryolite-specific selection before generic Unix
in the affected modules. Preserve existing upstream algorithms where possible.

| Area | Native implementation direction |
| --- | --- |
| Process entry and exit | Existing SysV stack and separate StartupRecord, mlibc TCB/static TLS and initialization/finalization |
| System allocator | Existing fallible user Talc heap through compatible C allocation hooks |
| Mutex, Condvar, Once, Parker | Upstream futex algorithms over native aligned private park/wake |
| Threads and TLS | Existing runtime-owned stacks, native completion and provider C11/TSS lifecycle through a narrow bridge |
| Monotonic time | Native clock/deadline calls with checked conversion |
| Files | Service-backed descriptors with shared descriptions, byte paths, checked I/O and explicit errors |
| Unix extension types | Cryolite-owned scalar and layout bindings; no Linux header substitution |
| Randomness | Real admitted entropy source; fixed bytes, timestamps and addresses are not entropy |

The first native profile keeps panic-abort. Unwinding, C++ exceptions and dynamic
linking are separate deliverables. Real-time clock support must not fabricate
UTC from the monotonic clock. Standard streams, environment, readiness and
process inheritance need explicit service/runtime contracts.

The ordinary Unix PAL currently assumes poll, fcntl, open('/dev/null'), dup,
signals and stack-overflow setup before main; Unix thread code uses pthreads.
The current ANSI/static provider does not supply that whole surface. A native
backend must deliberately bypass unsupported initialization and return truthful
errors for unsupported operations. A no-thread or panic-only fallback does not
prove native threading or clocks.

## Toolchain and validation

Build from the library sources matching targets/cryolite-toolchain.json. Stage
owned copies and patches under a generated build directory. Never patch the
installed compiler, its rust-src or the shared registry cache. A patched
experimental sysroot is distinct from upstream target acceptance.

The first working std probe must execute actual allocation, Vec/String,
Arc/Mutex, native creation/join, compiler TLS plus destructors, Instant and
sleeping in Ring 3. Test allocation failure, wait-credit pressure and service
failure as well as success. HashMap seeding requires the real random backend;
no deterministic security claim is allowed. Native file clones and Unix file
extensions follow the descriptor and service contracts.

BetAFS remains an independent std-based Ring 3 service. Its host-only FileDevice
and codec build need the resulting native platform surface; they do not become
kernel dependencies. Its compatibility harness remains host evidence until a
native executable and real storage path run successfully.

See [runtime direction](0019-native-runtime-and-standard-libraries.md),
[owned C ABI](0028-cryolite-owned-c-abi.md),
[file descriptions](0040-service-backed-files.md) and
[descriptor duplication](0042-descriptor-duplication.md).
