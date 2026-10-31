# User space

Services run as separate Ring 3 processes. Their client libraries use the native ABI. Drivers, filesystems, VFS, and networking remain outside `kernel/core`.

These directories are reserved locations, not Cargo packages or executable services. The user-space target, libc, and runtime remain undecided.

`filesystems/betafs/` is reserved for the independent BetAFS engine's Kryo adapter. Keep the engine in its own project.

See the [layout](../docs/layout.md) for component responsibilities and the [architecture](../docs/architecture.md) for service boundaries.
