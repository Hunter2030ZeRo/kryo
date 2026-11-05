# CPU random bootstrap validation

This record covers [ADR 0045](../../decisions/0045-cpu-random-bootstrap.md): an
optional CPU-only random source, register-return native calls and the bounded
private C/std bridge. The user accepted the initial CPU-only trust policy.

| Configuration | Result | Required checks |
| --- | --- | --- |
| Debug, Haswell-noTSX, one CPU, ABI/libc/std | Passed | 88 |
| Debug, Haswell-noTSX with RDRAND masked, one CPU, std | Passed | 69 |
| Release, max, four CPUs, normal supervisor + ABI/libc/std + disposable block read/write | Passed | 48 |

Build/output trees are isolated. The masked CPU still passes v3 admission and
the existing file/thread/TLS/quota checks. Its random requests return Unsupported,
clear both output registers and preserve bridge destination buffers. Supported
runs execute guarded-buffer random fills and functional HashMap operations.
The kernel independently compares the user completion event with the BSP CPU
feature state. Contradictory or forged completion lines are rejected.

The native probe rejects reserved arguments, checks cleared outputs on errors,
accepts empty requests without pointer/source access, rejects oversized bridge
requests and checks writes within canaries and an actual unmapped upper guard.
The ordinary fixture still requires final process, peer, object, wait, quota,
heap and backing retirement. The runner ends QEMU from the host; this does not
establish guest shutdown.

Host results: HAL 84 tests, interfaces 12 tests, SDK nine tests, 43 C/C++ header
checks with 109 native constants, 47 Python tests, formatting and Clippy with
warnings denied passed. Seven HAL tests exercise feature admission, CF failure,
finite retry bounds, successful zero, short words and discarded prefixes.
The verbatim C bridge passed 314 cases at O0 and O2 with ASan/UBSan, including
all bounded lengths, failures at every batch position, untouched destinations
and subsequent recovery. Independent review reran those cases and 34 parser
tests, inspected all three native records and found no blockers.

The debug image metadata was refreshed after clarifying its feature scope.
metadata-refresh.json confirms its tested executable hash did not change.
Archived build records include actual source pins and executable hashes.
After guarded application, both the kernel check and std executable rebuild
also passed from /home/entity27th/kryo; actual-verification.json records that
separate verification.

These checks establish instruction/status handling and runtime functionality
in QEMU TCG. They do not measure physical entropy quality, independence,
cryptographic seed strength, hardware performance or CPU trustworthiness.
No uniqueness, nonzero-value or statistical-output test is treated as evidence
of entropy. Source exhaustion is injected in host tests; it is not claimed as a
deterministic hardware failure reproduced in QEMU. Public libc getentropy/
getrandom, startup hardening, device sources and a broader runtime port remain
separate work. BetAFS and ICE are unchanged.
