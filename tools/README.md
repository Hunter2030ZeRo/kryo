# Development tools

The root [Makefile](../Makefile) provides build, ISO, test, and QEMU targets.

`boot_smoke.py` builds and boots a test ISO using headless QEMU TCG. It checks frame allocation, RAM zeroing, mapping/reclamation, and breakpoint return.

```sh
python3 tools/boot_smoke.py --profile debug
python3 tools/boot_smoke.py --profile release
python3 tools/boot_smoke.py --all
```

Each profile gets one boot attempt with a 30-second boot deadline. Logs and evidence are saved under `build/smoke/<profile>/`. The runner stops only the QEMU process it started and preserves output on failure.

`xtask/` and `debug/` are reserved for future tooling.
