# Experimental native Rust std

This port builds the pinned Rust library sources for an isolated Cryolite
target. It is a restricted static runtime profile, not an upstream Rust target
or a complete Unix/POSIX environment. The kernel and existing freestanding
services keep their current targets.

The port provides allocation through the existing fallible Talc heap,
Vec/String/Arc, native threads with requested stack sizes, join and detach,
compiler TLS with destructors, upstream futex synchronization, startup arguments
and monotonic time. Basic File/OpenOptions, read/write/seek, shared-cursor cloning
and descriptor conversion use the existing native file service. A narrow C bridge reuses the pinned mlibc C11 lifecycle;
it does not implement pthreads or use Linux system calls.

The std-linked allocator is an rlib in the same Rust compilation. Its `std`
feature omits the standalone global allocator and panic handler. System still
calls C malloc, which uses that one Talc heap. The separate C probe continues
to use the original standalone allocator archive.

## Build

Use the Rust/Cargo revisions in `sources.json`, the Clang version in
`targets/cryolite-toolchain.json`, installed matching rust-src and cached locked
dependencies. Prepare the pinned mlibc archives through its existing build
workflow. Then run:

```sh
python3 tools/build_std.py --profile debug --source-cache /path/to/libc/cache
python3 tools/boot_smoke.py --profile debug --std-probe
```

`CRYOLITE_LIBC_CACHE` supplies the source-cache directory to both commands.
The optional `--libc-source` argument selects the unmodified libc 0.2.191 crate
when it cannot be identified uniquely in Cargo's registry cache.

The build copies rust-src and libc into an owned output directory, verifies
their complete source hashes, applies the local overlay and records generated
source hashes. Changed generated sources are rebuilt; unowned directories are
preserved and rejected. The installed compiler, rust-src and registry sources
are never patched. Cargo's private source-root override is tied to the pinned
Cargo revision and must be reviewed when updating it.

The experimental target declares the Unix family but remains `std=false` and
requires `#![feature(restricted_std)]`. Its initial `std::os::unix` surface is
limited to byte strings, descriptor ownership and open options. Missing Unix extensions are
not imported from another operating system. Descriptor cloning starts at 3 and
sets CLOEXEC; descriptors 0, 1 and 2 are not automatically bound.

## Limits

- Basic file I/O uses byte paths within an explicitly granted namespace. Metadata,
  directories, positioned I/O, file-length mutation and storage synchronization
  remain unsupported. `stream_len` fails without moving a shared cursor.
  `File::flush` has no buffer to drain and does not imply durable storage.
- Network, environment mutation, process spawning, pipes and broad Unix
  extensions remain unsupported. BetAFS and its host FileDevice are not
  executable from this limited file surface alone.
- There is no entropy backend. RandomState/ordinary HashMap seeding and random
  APIs fail explicitly. Timestamps and addresses are not random seeds.
- Instant uses the native monotonic clock. SystemTime cannot supply wall-clock
  time. Available CPU count and native thread naming are not implemented.
- Panics abort. Stack unwinding, backtraces, dynamic linking/TLS, signals,
  pthreads and a hosted ABI release remain separate work.
- Infallible synchronization may yield and retry when wait credits are
  exhausted. Normal uncontended locking uses user atomics. This fallback is
  correctness behavior under quota pressure, not a latency or power guarantee.

The optional probe executes in Ring 3. The kernel checks actual private-word
parking while the mutex is held, successful join/detach and TLS destruction,
allocation failure and recovery, deadlines, then complete process/backing
retirement. Its trusted completion lines are separate from user diagnostics.
An additional process verifies direct native abort and its complete retirement.
These are QEMU functional checks, not hardware performance measurements.

The file probe uses the volatile validation service, not a mounted filesystem.
See [the file contract](../../docs/decisions/0044-native-std-files.md).
