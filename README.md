# Kryo

Kryo is the Rust `no_std` microkernel for Cryolite. It targets x86-64 and boots through Limine. General device drivers, filesystems, VFS, and networking belong in user-space services.

The kernel currently provides exception setup and diagnostics, boot memory discovery, physical frame allocation, and a BSP boot mapping primitive. See [implementation status](docs/status.md) for supported behavior and test results.

## Build

Requirements: Rust nightly, the `x86_64-unknown-none` target, Limine, xorriso, QEMU, and Python 3. The Makefile uses `/usr/share/limine` by default.

```sh
make check
make test
make iso
make run
make iso PROFILE=release
```

The default kernel is `target/x86_64-unknown-none/debug/kryo`; the ISO is `build/kryo.iso`.

## Test

```sh
make smoke
python3 tools/boot_smoke.py --all
```

`make test` runs host tests for the core and HAL. `make smoke` enables `boot-tests` and checks frame allocation, RAM zeroing, 128 map/unmap cycles, frame reclamation, and breakpoint return in QEMU. Normal boots omit these repeated checks.

Smoke output goes to `build/smoke/debug/` or `build/smoke/release/`. The runner preserves failure logs and does not retry automatically.

## Documentation

- [Architecture](docs/architecture.md)
- [Design contract](docs/design-contract.md)
- [Repository layout](docs/layout.md)
- [Implementation status](docs/status.md)
- [Roadmap](docs/roadmap.md)
- [Interface design](docs/interfaces.md)
- [Development guidelines](docs/development.md)

The workspace contains `hal`, `interfaces`, `kernel/core`, and `kernel/image`. User-space services and ICE adapters have reserved locations but no executable implementation in this repository.
