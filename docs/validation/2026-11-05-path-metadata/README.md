# Namespace metadata validation

Date: 2026-11-05. Baseline: 3aa083260981bd315ad09ed3137344b733907ee7.

PathInfo adds a dedicated namespace query for Rust metadata, symlink_metadata
and existence checks. It returns actual size/type/readonly properties without
opening a file, allocating a descriptor slot or creating a file gate. Follow
and NoFollow differ at the final symlink. See [ADR 0047](../../decisions/0047-path-metadata.md).

## Native execution

| Build | Configuration | Result |
| --- | --- | --- |
| Debug | Haswell-noTSX, one CPU, boot tests, ABI/libc/std probes | Passed |
| Release | max CPU, four CPUs, normal supervisor, ABI/libc/std, writable virtio block probe | Passed |

Both boots execute two normal std processes and one expected abort process.
Before file creation, queries report /probe absent and distinguish its link.
Further checks cover real path/descriptor metadata, snapshots after resize,
cursor preservation, readonly properties, root directory, followed links,
dangling targets and bounded loop errors. Permission denial propagates through
try_exists; Path::exists retains upstream error-suppression behavior. Empty,
NUL-containing, oversized and non-UTF-8 paths exercise their actual boundaries.

Repeated lookups preserve handle usage and return shared-page/wait usage to
baseline. The first open after these queries uses descriptor 3. Both existing
positioned-I/O workers now query metadata concurrently with their file I/O.
The one-credit case holds the sole wait credit: exists reports OutOfMemory,
then explicit lookup succeeds after release. Both configurations check zero
retained loan charge; the debug boot-test kernel also verifies the exact
CallExchange/OutOfResources failure. Previous file/data/cursor,
thread/TLS, abort and complete resource-retirement checks remain required.

The kernel checks event 237 in each normal process. The smoke parser requires
one final trusted RUSTSTDPATH marker after the full sequence; user diagnostics,
missing/duplicate markers and incorrect counts are rejected.

## Host checks and review

- The actual C provider body passed 81 behavioral groups at O0 and O2 under
  ASan and UBSan. Tests cover policy/path validation, allocation and transport
  failures, malformed replies, unexpected capabilities, unchanged outputs,
  namespace ownership, FD independence and interleaved window/table lifetime.
- The interface crate passed 16 tests. A separate host harness imports actual
  source: nine protocol tests and 12 resolver/storage-helper tests passed.
  It does not execute IPC dispatch; native probes cover that path.
- Workspace formatting and Clippy with all features passed, as did 49 Python
  tool tests. Final native builds compile/link the pinned provider and std port.
- A separate reviewer reran 81 provider groups at both settings and 21 backend
  tests, inspected source and both final native logs, and found no open issues.

Reproduce from the repository root with unique output directories:

```sh
rtk proxy python3 userspace/libc/mlibc/tests/file_runtime.py --output-dir /tmp/kryo-path-runtime
rtk proxy cargo test -p kryo-interfaces --target x86_64-unknown-linux-gnu --locked --offline
rtk proxy rustc --test --edition=2024 userspace/service-peer/tests/file_backend.rs -o /tmp/kryo-path-backend
rtk proxy /tmp/kryo-path-backend
rtk proxy python3 -m unittest discover -s tools
rtk proxy cargo fmt --all -- --check
rtk proxy cargo clippy --workspace --all-features --locked --offline -- -D warnings
rtk proxy python3 tools/boot_smoke.py --profile debug --cpu Haswell-noTSX --abi-probe --libc-probe --std-probe
rtk proxy python3 tools/boot_smoke.py --profile release --cpus 4 --normal --abi-probe --libc-probe --std-probe --block-probe
```

Use the pinned compiler, rust-src and offline archives documented by the std
profile. CRYOLITE_LIBC_CACHE selects that archive directory. Isolate concurrent
builds in separate source/output directories.

## Proof boundary

source-manifest.json records matching source inputs in both isolated builds;
probe build records retain compiler/library and image hashes. The actual
repository is separately checked and rebuilt after hash-guarded application;
actual-verification.json records that result. Archived log copies trim
trailing whitespace; original streams remain in the external work directory.
QEMU was stopped by the host after all markers; guest shutdown is not proved.

The service is a bounded volatile fixture. Directory iteration, general path
traversal, cwd/mount policy, file locks, timestamps and true durability remain
separate work. BetAFS FileDevice still needs locks and file/directory durability.
Neither independent BetAFS nor ICE sources were changed or executed. These
checks establish no hardware performance result or full POSIX/std compatibility.
