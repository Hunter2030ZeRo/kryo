# Native C calls and diagnostic snapshots

Date: 2026-11-04. Contract: [ADR 0029](../../decisions/0029-native-c-and-diagnostics.md).

## Implemented behavior

The [native C SDK](../../../userspace/libsys/c/README.md) directly invokes a
small subset of experimental Kryo ABI zero. Signed status and full-width values
remain separate. Experimental [C profile source headers](../../../userspace/libc/abi/README.md)
provide the reviewed types and constants without host-header fallback or an
installed sysroot.

DebugWrite, call 62, validates DebugConsole WRITE authority before inspecting
the byte range. A cold noninlined helper snapshots at most 256 bytes from owned
published user RAM while CPU execution is quiescent. It emits one escaped line
only after every read succeeds. Zero length reads nothing and logs nothing.
Invalid ranges and an unreadable second page produce no partial line.

The optional mixed-language ELF now executes native calls from C in Ring 3:

- Read the actual startup magic/version and a high-bit argument without
  mistaking a successful value for an error.
- Reject unknown calls, invalid handles, wrong object kind and missing WRITE
  authority. A temporary zero-rights duplicate is explicitly closed.
- Exercise yield, monotonic clock, elapsed sleep, private-word mismatch,
  elapsed wait, bounded wake and word alignment errors.
- Reserve two pages, verify zeroed commit, decommit/recommit and release, and
  reject diagnostics through a released mapping.
- Read RO image strings, escape binary controls and attempted kernel-marker
  text, and snapshot 256 bytes across two RW pages.

C emits three diagnostic records and the Rust wrapper emits a fourth. Their
payloads, original byte counts and common process identity are validated.
Diagnostic records are excluded from trusted kernel-marker matching. User
event 98 alone is insufficient: the CABI marker follows actual thread, object,
quota and frame retirement. Existing TLS, AVX2 preemption, private VM, atomic
wait, SMP and block-service assertions remain required.

## Verification

Six QEMU TCG configurations passed with 512 MiB guest RAM:

| Profile | CPU / processors | Mode | Modules |
| --- | --- | --- | --- |
| debug | Haswell-noTSX / 1 | boot-tests | ABI probe |
| release | Haswell-noTSX / 1 | boot-tests | ABI probe |
| debug | max / 4 | normal | ABI probe, writable block fixture |
| release | max / 4 | normal | ABI probe, read-only block fixture |
| debug | max / 4 | boot-tests | Earlier freestanding modules |
| release | max / 4 | normal | Earlier freestanding modules |

The final runs use the cold helper. Earlier passing runs before that structural
refactor are not included in this matrix. Both probe profiles remain within the
unchanged sixteen-LOAD-page policy; linked disassembly contains no MMX use.
The smoke runner stops its own emulator after required markers. Host termination
does not establish guest shutdown, hardware timing, scaling or power use.

The 453 existing Rust host tests passed: 354 core, 77 HAL, seven interfaces,
four libsys and eleven service tests. Seven separate diagnostic-helper tests
cover empty/invalid ranges, exact bounds, two-page reads, second-page failure,
control-byte escaping and maximum output. Python's 23 parser/packaging tests
include malformed diagnostics and attempted marker forgery. Formatting and
Clippy with warnings denied pass for normal/all-feature kernels, userspace and
the isolated native probe.

The header checker passes 24 checks: individual/repeated C11/C++23 includes,
both include orders and layouts, 248 profile constants, 40 native constants
compared with Rust, and four rejected profile changes. Its separate Linux-host
program checks all 65,536 packed wait-status values and single evaluation of
four macro arguments. Those are host macro checks, not Kryo process waits.

Reproduce the focused checks:

```sh
rtk proxy python3 tools/check_c_headers.py --outputdir /tmp/kryo-c-headers
rtk proxy rustc --edition=2024 --test docs/validation/2026-11-04-native-c/diagnostic-tests.rs -Dwarnings -o /tmp/kryo-diagnostic-tests
rtk proxy /tmp/kryo-diagnostic-tests
rtk proxy python3 tools/boot_smoke.py --profile debug --cpu Haswell-noTSX --abi-probe
```

[evidence.json](evidence.json) indexes final emulator runs and artifact hashes.
[header-validation.json](header-validation.json) records compiler versions,
commands, expected rejections and header inputs. [host-validation.json](host-validation.json)
records host counts and checks; raw test and emulator logs are retained.
[source-manifest.json](source-manifest.json) identifies source/configuration
inputs independently of documentation and generated artifacts.

## Remaining integration

No libc, malloc, stdio, Rust std, BetAFS or ICE program is built or executed by
this milestone. C runs inside the existing freestanding mixed-language ELF;
this is not an independent hosted C startup. The profile headers still lack
the full provider type closure, and no installed system header set is claimed.

The pinned mlibc allocator's OOM assertion, signal validation, clock visibility,
completion-aware pthread lifetime, legacy IFUNC paths, target compiler builtins
and actual native sysdeps remain explicit gates in ADR 0029. The framebuffer
rectangle remains early graphics output; these execution results appear in the
debug log. Run `make run ABI_PROBE=1` to include the new optional checks.
