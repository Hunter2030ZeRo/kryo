# Thread-to-process lookup validation

Date: 2026-11-04. Source base: bb5de2e.

Six changed image-runtime inputs are hashed in evidence.json. A separate
read-only review checked their final hashes, lifecycle ordering and fixtures.
All-feature kernel build, Clippy with warnings denied, formatting and 29 core
process/task tests pass.

Both QEMU TCG cases pass: debug/max/four CPUs with the native ABI and disposable
RW block probe; release/Haswell-noTSX/one CPU with the native ABI probe. The
guest logs and machine-readable checks are retained here. The new fixture
checks stale task/process generations, peer isolation, cleanup retry and final
detachment. The scale check compares 127 thread backlinks with canonical
membership after growth and rollback.

The matrix ran in an isolated source snapshot. The actual checkout is checked
against the reviewed hashes and rebuilt before commit. This does not measure
hardware latency or establish parallel kernel policy.
