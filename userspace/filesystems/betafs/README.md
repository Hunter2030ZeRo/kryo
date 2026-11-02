# BetAFS adapter

BetAFS is Cryolite's filesystem. This directory is reserved for its native Kryo
block/VFS adapter. The independent engine remains outside the Kryo repository.
There is no mounted filesystem, file service or executable adapter here yet.

The current engine's `betafs_block::BlockDevice: Send` uses:

```rust
fn block_count(&self) -> u64;
fn read_block(&self, id: u64, block: &mut [u8; 4096]) -> std::io::Result<()>;
fn write_block(&mut self, id: u64, block: &[u8; 4096]) -> std::io::Result<()>;
fn sync(&mut self) -> std::io::Result<()>;
```

The transport must preserve read-your-writes. Successful sync must make every
prior write durable; submission and interrupt completion alone are insufficient.
Failed write/sync may leave partial or uncertain storage. The engine poisons an
uncertain commit and requires reopening before further work.

For 512-byte sectors, check `id * 8`, capacity, buffer length and full completion.
Pass bulk data through bounded shared memory, retain its ownership through actual
device retirement, and reject stale service generations. Do not serialize Rust
pointers or `std::io::Error` values as IPC records.

The engine and its block/tree/codec layers currently require `std`. Before native
integration, port those dependencies in the independent project or provide the
required native runtime. Kryo must not duplicate the engine's format parser or
claim mount support from PCI discovery or a block-transfer test.

See [storage ownership](../../../docs/decisions/0017-pci-discovery-and-dma-storage.md)
for the current DMA and device-isolation gate.
