# User space

Services run as separate Ring 3 processes. Their client libraries use the native ABI. Drivers, filesystems, VFS, and networking remain outside `kernel/core`.

The separate user workspace builds the native supervisor, service clients,
drivers and execution probes. `libsys` wraps experimental version-0 calls;
`services` owns user-space policy and records. Component directories without a
Cargo package remain reserved locations.

The optional `abi-probe` uses a dedicated x86-64-v3 target for freestanding
C/C++/Rust calls, SysV startup, compiler static TLS and explicit private VM.
Reserve holds VA without data RAM; commit supplies zeroed RW/NX backing;
decommit/release return credit after retirement. User allocation and atomic
park/wake remain separate work. mlibc is the primary port candidate, not an
installed native sysroot; libc, libc++ and Rust std ports are incomplete.

`filesystems/betafs/` is reserved for the independent BetAFS engine's Kryo adapter. Keep the engine in its own project.

See the [layout](../docs/layout.md) for component responsibilities and the [architecture](../docs/architecture.md) for service boundaries.
