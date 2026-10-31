# Tests

Current tests run from the packages that own the implementation:

| Location | Coverage |
| --- | --- |
| `kernel/core/tests/range.rs` | Physical ranges, alignment, and overflow |
| `kernel/core/tests/normalize.rs` | Region filtering, reservations, and a generated page-level oracle |
| `kernel/core/tests/pool.rs` | Allocation ownership, exhaustion, reuse, initialization failure, and host-thread exclusion |
| HAL paging unit tests | Address arithmetic, physical width, table-frame validation, and page indices |
| `kernel/image/src/boot/checks.rs` | Guest RAM zeroing, mapping, TLB invalidation, reclamation, and failure paths |

Run host tests with `make test`. Run the guest checks with `make smoke`; they are gated by `boot-tests`.

The root `tests/` subdirectories reserve future boot, syscall, IPC, recovery, compatibility, and performance suites. Cargo does not discover tests here automatically.

Host models, QEMU behavior, and hardware measurements are separate forms of evidence. See [status](../docs/status.md) for recorded results.
