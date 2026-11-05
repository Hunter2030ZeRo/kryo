# Restricted native Rust standard library

Date: 2026-11-05. Status: experimental implementation.

Implement the first executable std profile through Cryolite-specific subsystem
dispatch, following [ADR 0041](0041-cryolite-rust-std.md). Keep the freestanding
target and normal service images unchanged. A separate v3 target exposes Unix
classification while retaining restricted-std admission and std=false metadata.

Use owned copies of the exact Rust library and libc crate sources. Verify full
input and generated source manifests before reuse. Do not modify the installed
toolchain or shared dependency cache. The private Cargo source-root override is
part of the pinned build contract, not a supported upstream target interface.

Reuse upstream futex Mutex, Condvar, Once, RwLock and Parker algorithms over
native aligned private park/wake. Wait-credit exhaustion permits a yield and
predicate retry. Mapping cancellation during a live Rust atomic borrow, invalid
wait results and failed wake operations are invariant failures. Sleep retries
quota exhaustion against the original absolute monotonic deadline.

A narrow mlibc bridge owns C11 thread creation, actual completion, detach
reaping and TSS callbacks. Rust initializes ThreadInit in the child and registers
its native TLS destructor list through provider TSS. Provider completion follows
TLS destruction and execution retirement, before join permits TCB/stack reuse.
Requested stack sizes use the pinned provider's internal attribute contract.

System allocation calls the existing C allocation interface. Link the Talc
adapter as an rlib with the same std build, omitting its standalone allocator
and panic handler. This avoids two Rust runtimes while preserving one heap for
C and Rust. The existing standalone C profile remains independent.

Do not select generic Unix startup, pthreads, files or sockets. The initial Unix
surface includes byte-string conversion and descriptor ownership only. General
file operations, inheritance, PTYs and BetAFS remain follow-ups. Absent entropy
must fail explicitly; no deterministic RandomState implementation is admitted.
Monotonic time does not supply UTC. Panics abort and backtrace capture has no
native backend.

The [profile](../../userspace/rust-std/README.md) records build instructions,
implemented APIs and limits. Native validation must include real RAM-quota
failure and recovery, contended kernel parking, timed waits, one-credit pressure,
join/detach, compiler TLS destruction and complete resource retirement. A
successful probe establishes only this restricted runtime boundary.
