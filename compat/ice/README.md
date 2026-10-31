# ICE adapters

This directory is reserved for Kryo integration with the independent ICE project.

- `linux/`: Linux ABI and execution semantics.
- `nt/`: NT ABI, PE loading, and handle semantics.
- `userspace/`: Compatibility loaders, libraries, and runtime support.

Both ABI paths use native syscalls, IPC, and services independently. NT support does not pass through ICE-Linux. Linux/NT dispatch and types do not belong in the kernel core.

There is no executable adapter here yet. macOS/XNU support is deferred.
