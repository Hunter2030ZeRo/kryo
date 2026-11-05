# Native BetAFS service

This adapter runs the independent BetAFS engine in Ring 3 and serves the native
file protocol. Kryo supplies execution, capabilities, memory and block IPC.
The engine and compression algorithms remain in their original project; the
build uses verified copies without modifying that working tree.

## Service contract

Startup requires service 60, instance 1 and four exact grants:

| Role | Kind | Rights |
| --- | --- | --- |
| 1 | DebugConsole | WRITE + DUP |
| 10 | Endpoint | RECEIVE + CONTROL |
| 20 | Endpoint | SEND, block reads |
| 21 | Endpoint | SEND, block writes and flush |

Clients receive namespace gates. Each open description owns an inode pin,
cursor, access/append state and advisory lock identity. Final gate closure
unpins the inode after accepted work ends. Failed final-unpin keeps its closed
record reachable; uncertain storage results quarantine the session and reject
pending lock requests. Mutations are not automatically replayed.

The adapter implements open/create, byte reads/writes, positioned I/O, seek,
descriptor/path metadata, resize, directory snapshots, advisory locks and
synchronization. Byte components use the engine lookup API. The current engine
supports files and directories; symlinks, general mount routing, cwd handling
and namespace mutation RPCs remain separate work.

Sync Data and Sync All both use FileSystem::sync, including a completed block
FLUSH. This provides a stronger full-filesystem barrier for the data-only
request. Read-only directory handles can synchronize namespace state.
Compression uses the engine default. All engine codecs are built: Raw, Zstd,
LZ4 and OpenZL. The native codec probe requires actual codec selection and
decoding, then reopens engine files encoded with each codec.

## Storage ownership

BlockDevice uses fixed 4096-byte units. The serialized adapter validates exact
counts, request IDs, service instance and statuses. Only the device transport
converts block IDs to 512-byte sectors. Fresh backing per request remains owned
through accepted/canceled operations; it is not reused after uncertain delivery.

The current validation layout assigns a whole disposable scratch disk and uses
physical blocks 1 through N-1 for the filesystem. Block zero retains the
existing scratch marker. Capacity and offset addition are checked. General
device admission and partition policy remain future service-manager work.

Startup argument bit 0 explicitly selects format of zero superblocks and seeds
the validation namespace; otherwise the service opens an existing filesystem.
Bit 1 enables validation-only barrier events. Normal synchronization adds no
debug syscalls. Open never falls back to format. A production launcher must
deliberately assign storage and phase.

## Build and verification

Use the pinned Cryolite Rust/C toolchains, rust-src and offline dependencies.
Build tools create their own copies of the engine, platform sources and required
registry packages. They verify archives, source bytes and lock resolution before
building. OpenZL receives scoped cross-build recipe changes for its C API;
algorithm sources stay unchanged. Installed toolchains and registry sources
are not patched. The executable uses its own ELF link address and a bounded
larger initial stack.

```sh
rtk proxy python3 tools/build_betafs.py --betafs-source /path/to/betafs --source-cache /path/to/libc/cache --manifest-path userspace/filesystems/betafs/Cargo.toml --build-dir build/betafs-service --output build/betafs-service.elf
rtk proxy python3 tools/check_betafs.py --betafs-root /path/to/betafs --outputdir build/betafs-frontend-check --frontend
rtk proxy python3 tools/boot_betafs.py --betafs-source /path/to/betafs --source-cache /path/to/libc/cache --outputdir build/betafs-two-boot --profile release --cpu max --cpus 4 --normal
```

The committed manifest is an input template for the build tool. The generated
manifest supplies verified engine and platform dependencies in an owned build
directory. Builds preserve the external working tree, including existing edits.

The boot verifier creates one owned disk, formats/writes/synchronizes in the
first guest, kills QEMU after completion and opens the same disk in a second
guest. It never recreates the disk between boots or reformats an existing test
output. Kernel checks pair each service sync with a new successful VirtIO FLUSH.
Separate clients test blocked lock recovery after the owner process is stopped.

Host fault injection, guest execution and physical hardware are separate proof
boundaries. QEMU verifies the implemented flush/reopen path; hardware power-loss
and performance measurements remain pending. See [ADR 0049](../../../docs/decisions/0049-native-betafs-durability.md)
and the [validation record](../../../docs/validation/2026-11-05-durable-files/README.md).
