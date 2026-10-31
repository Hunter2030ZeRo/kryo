# Development tools

The root [Makefile](../Makefile) provides build, ISO, host-test, and QEMU targets.

`boot_smoke.py` builds a test ISO and runs it once under headless QEMU TCG:

```sh
python3 tools/boot_smoke.py --profile debug
python3 tools/boot_smoke.py --profile release
python3 tools/boot_smoke.py --all
```

Required markers cover memory allocation/zeroing, mapping and VM rollback, 16 user-isolation cases, eight timer-driven switches, capability rejection, invalid saved-state isolation, pending-interrupt drain, and full task teardown.

IPC adds eight rendezvous/cancellation/failure/notification cases and three BSP halt/wakeup cases. These exercise the experimental 32-byte inline protocol; capability transfer and a service runtime are pending. The run ends with the ABC breakpoint sequence.

Each profile has a 30-second host boot deadline. Logs and JSON evidence are saved under `build/smoke/<profile>/`. Failure output is preserved; the runner does not retry automatically. It stops only its own QEMU process after the checks or deadline.

Timer checks use fixed xAPIC counter values. The host deadline is independent of the guest's uncalibrated timer quantum. STI/HLT/CLI checks establish guest wakeup behavior, not physical power savings. Host termination of QEMU does not prove guest shutdown.

`xtask/` and `debug/` are reserved for future tooling.
