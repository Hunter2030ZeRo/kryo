# BetAFS service compatibility

Date: 2026-11-05. Kryo base: `8e0075710a2164c5097f4e9a0c1bf914799d0061`.
BetAFS tree: `7dc43ef438269c8a307f77b8af9002fb4a10918f` plus the existing local
changes recorded by source hashes. The independent tree was not modified.

## Conclusion

The current block and file APIs provide compatible boundaries for an independent
BetAFS Ring 3 service. No filesystem engine or new filesystem policy was added
to Ring 0. Native BetAFS execution is not established: Cryolite Rust/std, native
codec dependencies and general block-device admission are still required.

| Evidence | Result | What it establishes |
| --- | --- | --- |
| Real BetAFS engine over a host wire model | 15 tests passed | Existing engine accepts a custom BlockDevice using Kryo block units and message definitions |
| Verifier output containment | 5 tests passed | Reused output cannot redirect managed writes through external symlinks |
| Existing Python tool checks | 37 passed | Verification additions preserve the current tool checks |
| QEMU release/max/4 CPU, writable scratch disk | Passed; disk hash verified | Actual Ring 3 block read/write/Flush and retirement path |
| QEMU release/max/4 CPU, read-only scratch disk | Passed; disk unchanged | Actual write denial and resource retirement |

The host harness links actual betafs-block/core/tree crates and the actual
kryo-interfaces crate. It does not copy their implementations. All 91 tracked
and untracked BetAFS inputs and its pre-existing dirty Git status remained
unchanged. Dependency resolution ran offline from a copied lock and preserved
resolved upstream versions/checksums. The current unconditional codec build,
including Zstd and OpenZL, compiled without source patches or feature removal.
This does not test native codec execution or compression performance.

## Host engine checks

The real engine formats storage, creates a file, writes 12,345 bytes in native-RPC
sized requests, reads partial data/EOF, truncates, syncs and reopens. Byte-based
names and creation metadata use the engine's real APIs. Pin/unlink/final-unpin
checks show how an open description can retain an inode after namespace removal.
The approved server-owned cursor and append contract maps to metadata/read_at/
write_at. These API checks are not an implementation of a native file service.

The test-only BlockDevice uses the native block decoder and explicit response
validation. Tests reject wrong request/instance identity, short data completion,
nonzero Flush completion, stale requests and wrong gate authority. Block IDs
stay in 4096-byte units; only the native device transport converts to sectors.

The model separates visible and durable blocks, so writes are readable before
Flush while a modeled crash loses unflushed changes. Failed publication writes
and barriers reach the actual engine's CommitUncertain/NeedsReopen behavior.
Accepted transport failures prevent replay; undelivered failures can retry with
fresh model backing. This proves the tested adapter/model semantics, not native
IPC buffer revocation, asynchronous DMA retirement or physical durability.

## Native boundary and remaining gates

The two QEMU runs execute the existing separate Ring 3 block driver and clients
using disposable disks. They check actual read/write/Flush, reduced memory
rights, malformed requests, accepted-cancellation loan retention and orderly
cleanup. QEMU FLUSH completion is functional command-path evidence, not an SSD
power-loss experiment. BetAFS is not loaded in either guest.

The current native block driver deliberately checks a fixed
CRYOLITE-BLOCK-PROBE-V1 marker in block zero. A real engine-formatted image does
not carry that marker. The host test checks this mismatch; this work preserves
the guard rather than overwriting a superblock or widening device admission.

The Cryolite bootstrap target still has std=false. The engine's Unix FileDevice
backend and unconditional codec build need native platform support or host-backend
separation. The kernel should not absorb these user-space dependencies. File
RPCs provide regular Open/Read/Write/Seek only; full VFS, directory/stat/fsync,
access policy and service recovery remain separate integration work. File and
block wire/status values require explicit translation.

## Reproduction

From the Kryo repository, using an existing local BetAFS tree:

```text
rtk proxy python3 tools/check_betafs.py --betafs-root /path/to/betafs --outputdir /tmp/kryo-betafs-contract
rtk proxy python3 -m unittest discover -s tools/betafs_contract -p test_output.py
rtk proxy python3 tools/boot_smoke.py --profile release --cpu max --cpus 4 --block-probe
rtk proxy python3 tools/boot_smoke.py --profile release --cpu max --cpus 4 --block-probe --block-readonly
```

The verifier owns only its marked output directory. Existing unowned output and
external-resolving symlinks are rejected before any managed write. The review
found an earlier reused-output symlink escape; five disposable regressions now
cover file, directory, owner and dangling-link cases. No independent engine file
was affected by that reproduction.

host-evidence.json records current source, lock, toolchain, commands and log
hashes. results.json records native evidence and archived file hashes. Source
hashes qualify the dirty BetAFS tree; a commit ID alone does not identify it.
[The boundary contract](../../../userspace/filesystems/betafs/README.md) maps
these results to the future thin service adapter.
