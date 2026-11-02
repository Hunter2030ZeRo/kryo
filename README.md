# Kryo

Kryo is the Rust `no_std` microkernel for Cryolite. It targets x86-64 and boots through Limine. General device drivers, filesystems, VFS, and networking belong in user-space services.

The kernel provides owned memory, shared pages, SMP user execution, calibrated timer preemption, and IPC with capability delegation and deadlines. Metadata grows through fallible allocation within configured service accounts: handles, waits, shared payload pages and live threads retain charges through retirement, while VM mappings/tables have per-process bounds. CPU storage and multiword masks cover 255 directed xAPIC identities; [implementation status](docs/status.md) records verified guest inventories. Threads execute with private stacks, TLS, and FP/SIMD state. Restricted device registers and a real HPET interrupt connect to a Ring 3 probe through owned handles. The BSP coordinates native calls and memory retirement between execution batches. Normal boot starts a native supervisor with versioned startup grants, role allowlists, private discovery gateways and service replacement.

BetAFS is the filesystem target. An opt-in Ring 3 driver now validates real
VirtIO block read/write/flush through private DMA queues. The persistent block
service, BetAFS adapter and native standard-library ports remain pending. See
[the storage boundary](docs/decisions/0018-mediated-virtio-block-transport.md).

## Build

Requirements: Rust nightly, the `x86_64-unknown-none` target, Limine, xorriso, QEMU, Python 3, and llvm-objcopy or GNU objcopy. The Makefile uses `/usr/share/limine` by default.

```sh
make check
make test
make iso
make run
make iso PROFILE=release
```

The default kernel is `target/x86_64-unknown-none/debug/kryo`; the ISO is `build/kryo.iso`.

`make user` builds eleven boot ELFs, including the supervisor, echo service and registry clients, alongside the legacy execution/device/growth probes. Packaging strips debug sections from boot copies and verifies that entry/load data is unchanged; full debugging artifacts remain under `build/userspace/`. Normal boot runs the configured service graph and keeps the supervisor and replacement service alive. `boot-tests` checks rejected startup, authority distribution and legacy init/worker behavior, then retires all test processes.

## Test

```sh
make smoke
python3 tools/boot_smoke.py --all
python3 tools/boot_smoke.py --all --cpu max
python3 tools/boot_smoke.py --all --cpus 8 --cpu max
python3 tools/boot_smoke.py --normal --profile release --cpus 65 --cpu max
```

`make test` runs host tests for core, HAL and interfaces. User-space policy and startup decoder tests run with `cargo test --manifest-path userspace/Cargo.toml -p kryo-libsys -p kryo-services --target x86_64-unknown-linux-gnu --locked --offline`. `make smoke` checks memory ownership, isolation, preemption, authority, IPC, halt/wake, service startup/replacement, growing registries and reclamation in QEMU. Use `python3 tools/boot_smoke.py --normal --all` to verify persistent user-space supervision in normal debug/release boots. Both modes use the shared image runtime.

Smoke output goes to `build/smoke/debug/` or `build/smoke/release/`. The runner preserves failure logs and does not retry automatically.

## Documentation

- [Architecture](docs/architecture.md)
- [Design contract](docs/design-contract.md)
- [Repository layout](docs/layout.md)
- [Implementation status](docs/status.md)
- [Roadmap](docs/roadmap.md)
- [Interface design](docs/interfaces.md)
- [Development guidelines](docs/development.md)

The workspace contains `hal`, `interfaces`, `kernel/core`, and `kernel/image`. Native user programs build separately under `userspace/`. General drivers, filesystem services, and executable ICE adapters remain pending.

Frame-allocation metadata is sized from Usable RAM during boot and reserved before
the heap starts. The earlier fixed 4-GiB pool limit is removed. See
[the memory contract](docs/decisions/0013-runtime-frame-metadata.md) and
[RAM validation](docs/validation/2026-11-01-ram/README.md) for supported evidence.
