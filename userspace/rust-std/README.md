# Experimental native Rust std

This port builds the pinned Rust library sources for an isolated Cryolite
target. It is a restricted static runtime profile, not an upstream Rust target
or a complete Unix/POSIX environment. The kernel and existing freestanding
services keep their current targets.

The port provides allocation through the existing fallible Talc heap,
Vec/String/Arc, native threads with requested stack sizes, join and detach,
compiler TLS with destructors, upstream futex synchronization, startup arguments
and monotonic time. Basic File/OpenOptions, read/write/seek, shared-cursor cloning
and descriptor conversion use the existing native file service. Descriptor
metadata, positioned I/O and resize extend that same service contract. Path
metadata and existence checks use explicit namespace lookups with final-symlink
policy. Directory iteration and advisory file locks use owned description gates.
A narrow C bridge reuses the pinned mlibc C11 lifecycle;
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
limited to byte strings, descriptor ownership, open options and FileExt I/O. Missing Unix extensions are
not imported from another operating system. Descriptor cloning starts at 3 and
sets CLOEXEC; descriptors 0, 1 and 2 are not automatically bound.

## Limits

- File I/O uses byte paths within an explicitly granted namespace. Descriptor
  metadata returns actual length, type and service readonly properties. FileExt
  positioned I/O and resize preserve shared cursors; positioned writes honor the
  supplied offset on append descriptions. `stream_len` uses one metadata query
  (and retains its upstream feature gate).
- Path metadata distinguishes follow/no-follow and propagates lookup errors.
  Directory entries own raw names and metadata snapshots; blocking advisory
  locks wait in IPC and survive descriptor aliases. Timestamps, permission
  mutation and the newer Dir traversal API remain unsupported.
- Sync All/Data reaches the backend. The volatile peer returns Unsupported;
  the BetAFS service waits for actual engine and block barriers. Directory
  handles support synchronization. `File::flush` does not imply durable storage.
- Network, environment mutation, process spawning, pipes and broad Unix
  extensions remain unsupported. BetAFS uses its native BlockDevice adapter;
  this profile does not enable all host programs or their runtime contracts.
- RandomState/HashMap uses the optional BSP CPU random source. The initial
  profile trusts CPU RDRAND output alone; unsupported hardware or temporary
  exhaustion fails explicitly. The infallible std API panics on such failure.
  Independent suppliers and hardware entropy assessment remain separate work.
  Timestamps and addresses are not random seeds.
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

The CPU source and its trust boundary follow [ADR 0045](../../docs/decisions/0045-cpu-random-bootstrap.md).

See [descriptor metadata and positioned I/O](../../docs/decisions/0046-descriptor-metadata-and-positioned-io.md) for the extension and its durability boundary.

See [namespace metadata lookup](../../docs/decisions/0047-path-metadata.md),
[directories and locks](../../docs/decisions/0048-directories-and-file-locks.md)
and [native BetAFS durability](../../docs/decisions/0049-native-betafs-durability.md)
for the service contracts and validation limits.
