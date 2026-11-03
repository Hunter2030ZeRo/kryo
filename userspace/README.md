# User space

Services run as separate Ring 3 processes. Their client libraries use the native ABI. Drivers, filesystems, VFS, and networking remain outside `kernel/core`.

The separate user workspace builds the native supervisor, service clients,
drivers and execution probes. `libsys` wraps experimental version-0 calls;
`services` owns user-space policy and records. Component directories without a
Cargo package remain reserved locations.

The optional `abi-probe` uses a dedicated x86-64-v3 target for freestanding
C/C++/Rust calls, SysV startup, compiler static TLS and explicit private VM.
Reserve holds VA without data RAM; commit supplies zeroed RW/NX backing;
decommit/release return credit after retirement. Libsys also wraps private
32-bit compare-before-park and bounded wake with absolute deadlines and mapping
invalidation. Users publish the predicate with a Release store and recheck with
an Acquire load; wake has no buffered event. User allocation and shared waits
remain separate work. mlibc is the primary port candidate, not an
installed native sysroot; libc, libc++ and Rust std ports are incomplete.

ThreadCreateNative borrows runtime-prepared stack/TLS ranges and installs FS
before the first child instruction. `libsys::ThreadMemory` describes those
ranges and an optional unmapped lower guard. Admitted VM retirement conflicts
return Busy until actual completion; native join permits reuse without kernel reaping
or automatic unmapping. The runtime supplies its TCB, startup and cleanup;
the kernel preserves fresh private CPU state. The probe demonstrates runtime
rounding-control inheritance, not a complete pthread implementation. See
[the ownership contract](../docs/decisions/0027-runtime-owned-thread-memory.md).

`filesystems/betafs/` is reserved for the independent BetAFS engine's Kryo adapter. Keep the engine in its own project.

See the [layout](../docs/layout.md) for component responsibilities and the [architecture](../docs/architecture.md) for service boundaries.
