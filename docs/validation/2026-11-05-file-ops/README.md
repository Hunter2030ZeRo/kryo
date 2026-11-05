# Descriptor metadata and positioned I/O validation

Date: 2026-11-05. Baseline: b8c3ae6d78540ed04bab9b7ea1b26199e5738d79.

The native file service now supports Info, ReadAt, WriteAt, SetLen and Sync.
The C provider and restricted Rust std profile use those requests without
changing the kernel syscall ABI. Descriptor metadata reports actual file
length, kind and service readonly policy. Path metadata and timestamps remain
unsupported. See [ADR 0046](../../decisions/0046-descriptor-metadata-and-positioned-io.md).

## Native execution

| Build | Guest configuration | Result |
| --- | --- | --- |
| Debug | Haswell-noTSX, one CPU, boot tests, native ABI/libc/std | Passed |
| Release | max CPU, four CPUs, normal supervisor, native ABI/libc/std, writable virtio block probe | Passed |

Each boot executes two normal std processes and one expected abort process.
The file checks cover genuine metadata snapshots, readonly object policy,
stream length without seeking, cursor preservation, EOF at the maximum signed
offset, shrink/regrow zeroing, capacity failure and resource retirement.
WriteAt honors its offset on an append description; ordinary Write appends.
Writable append descriptions may resize. Both synchronization scopes return
Unsupported from the volatile peer.

The first process also runs two positioned-I/O workers through cloned
descriptors. The second retains a one-wait-credit account: a helper holds that
credit while the main thread attempts WriteAt. The kernel verifies that
CallExchange fails with OutOfResources and no retained loan charge. After
releasing and joining the helper, the program verifies unchanged data, retries
explicitly and checks successful I/O without cursor movement. The kernel
validates events 232–236; the smoke parser requires the final trusted markers.

## Host checks

- The actual provider body passed 66 behavioral groups at O0 and O2 under
  AddressSanitizer and UndefinedBehaviorSanitizer. Native-call and allocation
  doubles cover malformed responses, poisoning across aliases, close/reuse,
  signed bounds, output preservation and unsupported barriers.
- The interface crate passed 15 tests. A separate harness imports the actual
  protocol and storage helpers: eight protocol tests and seven backend tests
  passed. These helper tests do not exercise IPC dispatch.
- The private std adapter compiled; the final guest builds compiled and linked
  the complete updated probes. Workspace formatting, Clippy with all features
  and 48 Python tool tests passed.
- An independent reviewer reran both sanitized provider variants and the
  backend harness, checked both final QEMU logs and found no open issues.

Reproduction commands from the repository root (output paths must be unique):

```sh
rtk proxy python3 userspace/libc/mlibc/tests/file_runtime.py --output-dir /tmp/kryo-file-runtime
rtk proxy cargo test -p kryo-interfaces --target x86_64-unknown-linux-gnu --locked --offline
rtk proxy rustc --test --edition=2024 userspace/service-peer/tests/file_backend.rs -o /tmp/kryo-file-backend-tests
rtk proxy /tmp/kryo-file-backend-tests
rtk proxy python3 -m unittest discover -s tools
rtk proxy cargo fmt --all -- --check
rtk proxy cargo clippy --workspace --all-features --locked --offline -- -D warnings
rtk proxy python3 tools/boot_smoke.py --profile debug --cpu Haswell-noTSX --abi-probe --libc-probe --std-probe
rtk proxy python3 tools/boot_smoke.py --profile release --cpus 4 --normal --abi-probe --libc-probe --std-probe --block-probe
```

The native builds require the pinned sources and offline dependency archives
described by the std profile. Set CRYOLITE_LIBC_CACHE to that archive directory.
Run separate builds in separate checkouts or output directories.

## Failure retained and proof boundary

The initial debug fixture required successful concurrent IPC with only one
wait credit. It failed with ENOMEM; the diagnostic run confirmed exhausted wait
authority. The final test separates concurrent success from deterministic
quota rejection and recovery. No production retry loop or quota increase hides
the failure. Initial logs and image build records are retained as initial-join-*
artifacts; the original packaged images remain in the external work directory.

source-manifest.json records matching source inputs in the debug and release
checkouts. Build metadata records pinned compiler/library inputs and image
hashes. QEMU was stopped by the host after all required markers; this does not
prove guest shutdown. The real repository is separately checked and rebuilt
after hash-guarded application; actual-verification.json records that result.
Archived log copies trim trailing whitespace; original build streams remain
in the external work directory.

The file peer is volatile. This is not persistent-storage, hardware,
performance, complete POSIX or full std evidence. Sync succeeds only when a
future backend completes its required storage barrier; no durability is claimed
here. BetAFS and ICE were not modified or executed by this validation.
