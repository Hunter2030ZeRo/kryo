# Development guidelines

Use short, descriptive names and functions with one clear responsibility. Keep failure paths explicit. Add abstractions and APIs when an implemented feature needs them.

Comments should explain safety requirements, invariants, or decisions that the code cannot express. Do not repeat the code in prose. Document ownership, mapping validity, and aliasing requirements at unsafe boundaries.

Follow the [design contract](design-contract.md) for resource lifetime and the [architecture](architecture.md) for subsystem boundaries.

## Changes

Each change should implement and verify a concrete completion criterion. Add tests for new invariants and failure paths. Record supported behavior and remaining limits in [status](status.md).

Allocation failure, quota exhaustion, invalid user input, and peer termination are ordinary errors. A violated boot invariant may halt the kernel after reporting diagnostics.

## Checks

```sh
make check
make test
make smoke
```

Boot regression tests use the `boot-tests` feature. Keep them out of normal startup.

Host tests validate data structures and state transitions. QEMU tests validate guest behavior. Hardware performance, power, and recovery results require separate measurements.
