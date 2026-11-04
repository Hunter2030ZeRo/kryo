# Native C thread validation

Date: 2026-11-04. Scope: native v0 C transport and runtime-owned thread backing,
not pthreads, a hosted libc or kernel SOF.

The C SDK exposes existing creation, exit, join and timed-join calls. The Ring 3
fixture creates two successive C children with caller-owned stack/TLS ranges.
It checks initial stack state, argument and FS delivery, atomic park/wake,
timeout without premature reclamation, Busy retirement, high-bit exit codes,
separate termination reasons, retained completion and reuse after actual join.
The combined probe retires 13 threads. Kernel success text follows zero remaining
account charges, object references and owned process frames.

## Results

| QEMU TCG configuration | Result |
| --- | --- |
| Haswell-noTSX, 1 CPU, debug boot tests | Passed |
| Haswell-noTSX, 1 CPU, release boot tests | Passed |
| max, 4 CPUs, debug normal boot, writable disposable block device | Passed |
| max, 4 CPUs, release normal boot, read-only disposable block device | Passed |

[Evidence](evidence.json) links each retained run. CTHREAD requires an exact
kernel retirement marker; event 99 or text inside USERDIAG cannot satisfy it.
The existing private-word idle proof excludes the earlier C thread phase.

The header checker passes 32 checks: C11/C++23 layouts/constants plus eight
thread-wrapper compilations spanning O0/O2, static/PIC and frame-pointer builds.
The compiler fixture keeps R12/R13/R14 values live across creation and uses all
join outputs. Python runner tests pass 24 cases. Core/HAL/interfaces and
libsys/services Rust host suites pass. Kernel all-feature and ABI-probe Clippy
pass with warnings denied.

After applying the reviewed source, 1,372 actual-checkout file hashes matched.
The actual source passed the header/Python checks, formatting, normal/all-feature
kernel builds and debug/release ABI builds with generated outputs relocated.
Both linked probes contain no MMX instructions. These actual rebuilds are
separate from the QEMU runs over identical staged source; neither demonstrates
hardware latency, power efficiency or a complete libc runtime.

## Initial failures and corrections

The first debug image occupied 17 load pages and was rejected by the existing
16-page loader bound before C execution. The bound is now 32 pages, retaining
segment/header/module checks and account mapping/table limits.

The first release run reached the C children but returned failure 443: the test
expected a successful timed join with expired deadline zero. Native admission
checks expiry before terminal-result observation. The retained-result check now
uses ordinary join. Kernel timeout ordering was unchanged.

Raw failed-run evidence remains under `first-attempt/`. All four final cases
above use the corrected source. Source/build hashes identify the checked inputs.

## Reproduction

```sh
rtk proxy python3 tools/check_c_headers.py --output /tmp/kryo-c-threads-headers
rtk proxy python3 -m unittest discover -s tools
rtk proxy python3 tools/boot_smoke.py --all --cpu Haswell-noTSX --cpus 1 --abi-probe
rtk proxy python3 tools/boot_smoke.py --profile debug --cpu max --cpus 4 --normal --abi-probe --block-probe
rtk proxy python3 tools/boot_smoke.py --profile release --cpu max --cpus 4 --normal --abi-probe --block-probe --block-readonly
```

Run guest builds serially in an isolated checkout. Disposable block images are
created and checked by the runner. Kernel targets are isolated per smoke run.
Recorded absolute paths in raw evidence describe the original build workspace.
