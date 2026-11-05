# Native std file validation

The [basic std file adapter](../../decisions/0044-native-std-files.md) executes
against the existing volatile Ring 3 file peer. No filesystem engine or wire
operation was added.

| Configuration | Result | Required checks |
| --- | --- | --- |
| Debug, Haswell-noTSX, one CPU, boot tests + ABI/libc/std | Passed | 87 |
| Release, max, four CPUs, normal supervisor + ABI/libc/std + disposable block read/write | Passed | 47 |

Separate source/output trees isolate the builds. The archived metadata includes
the actual probe source hashes, pinned std/provider inputs and packaged image
hashes. The runner terminates QEMU from the host after completion; it does not
prove guest shutdown.

Each guest executes the file test in two fresh std processes. The checks cover
fs::write/read/read_to_string with payloads beyond one 4096-byte loan, invalid
UTF-8, shared-cursor clones versus independent opens, File/OwnedFd ownership
conversion, seek overflow without cursor mutation, append and truncate-on-open.
Creation conflicts, denied write access, missing paths, invalid access options
and embedded NUL return errors. Metadata, set_len and sync_all must explicitly
return Unsupported.

Files close before the user completion event. The kernel then requires client
and peer exit, final gate closure, no live channels/words/waits/objects, zero
account charges and restored backing/heap baselines before publishing the
trusted RUSTSTDFILE marker. The peer enforces its existing successful-open and
closure counts. Existing std threading, quota recovery, TLS destruction and
native-abort checks still run in the same guest.

The complete std/probe check, workspace formatting, Clippy with warnings denied
and 45 Python tests passed. Independent review inspected the ownership, unsafe
buffer boundaries, flags and both native records and found no blockers.

This proves the bounded service-backed transport and std adapter in QEMU TCG.
The validation peer has a volatile 16-KiB backend with fixture-specific paths
and limits. It is not mounted BetAFS, production namespace/permission policy,
hardware performance, storage durability or failure recovery. File::flush is
unbuffered completion only; metadata, positioned I/O and durable synchronization
still need explicit service contracts.
