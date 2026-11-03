# Runtime-owned thread memory

Date: 2026-11-03. Contract: [ADR 0027](../../decisions/0027-runtime-owned-thread-memory.md).

ThreadCreateNative, call 61/version 1, accepts a raw entry, argument, stack and
TLS byte ranges, FS base and optional lower guard. Inputs travel in scalar
registers; no user descriptor pointer is dereferenced. The runtime prepares
these mappings. Kryo allocates the kernel entry stack and CPU state, validates
the inputs and retains one execution-range record per thread before publication.
The existing managed ThreadCreate call 32 remains available.

## Validation

Six QEMU TCG configurations passed with 512 MiB guest RAM:

| Profile | CPU model / processors | Mode | Optional modules |
| --- | --- | --- | --- |
| debug | Haswell-noTSX / 1 | boot-tests | ABI probe |
| release | Haswell-noTSX / 1 | boot-tests | ABI probe |
| debug | max / 4 | normal | ABI probe, writable block fixture |
| release | max / 4 | normal | ABI probe, read-only block fixture |
| debug | max / 4 | boot-tests | legacy modules |
| release | max / 4 | normal | legacy modules |

The four ABI runs migrate the two compiler-TLS children to call 61. Their parent
prepares the TCB/TLS slots before creation and reserves ten pages: eight backed
stack pages and two guard holes. Each child's FS is installed before its first
instruction. The runtime trampoline installs captured x87/MXCSR rounding controls
before calling Rust; TLS and rounding checks survive yield and sleep. The parent
restores its own rounding controls.

A readiness/release barrier keeps both children alive while the parent verifies
that guard commit, stack decommit, static TLS unmap and reservation release fail
without retiring their memory. Native join then permits clearing/reclaiming the
user resources, returning private/reserved usage to zero before the existing VM
probe. Event 97/1 records those checks. Static ELF TLS reaches the core Busy
check; legacy MemoryMap/MemoryUnmap still reject reserved VM arenas earlier with
InvalidArgument. This preserves their existing admission precedence.

The eight legacy word-test children remain, preserving managed TLS retirement
coverage and eleven total thread retirements. Existing SysV C/C++/Rust calls,
compiler TLS, private VM, atomic wait/HLT, live YMM preemption, SMP, resource
budgets and block-service checks remain required. The packaged ABI images use
16 LOAD pages in debug and 15 in release, within the unchanged 16-page policy.

Boot-test builds additionally run a trusted native-creation fixture. It checks
malformed scalar inputs, duplicate live ranges, SetTls confinement, control-object
exhaustion after lease acquisition, legacy/native coexistence and final frame
accounting. A stopped child's user stack is unmapped and reused before its
kernel resources reap; reaping must preserve the new contents. These are trusted
kernel state/cleanup tests, separate from the actual Ring 3 ABI execution.

All 453 Rust host tests and 20 Python tests passed. Core models cover private RW
admission, packed TLS blocks sharing a page, guard mapping exclusion, full-range
Busy preflight before callbacks, wrong-space/double release, dropped-token
retention, metadata OOM and lifetime-ID exhaustion. A four-TiB guard with two
actual data pages checks that reservation search skips the entire obstacle;
it is a sparse address-space model, not physical RAM validation.

Formatting and Clippy with warnings denied passed for normal/all-feature kernels,
scoped changed core tests, HAL host targets, the userspace workspace and the
custom-target ABI probe. [host-validation.json](host-validation.json) records
commands, package counts and source hashes. [evidence.json](evidence.json)
records six emulator runs and kernel/ELF/disk hashes; raw logs are retained.
[source-manifest.json](source-manifest.json) identifies source/configuration and
runner inputs, excluding documentation and generated files. The runner stops
its own emulator after required markers; this does not establish guest exit.

## Lifetime and scope

Leases protect mappings and guard holes, not the bytes against writes by sibling
threads. They are byte-range records without per-page allocation. Page retirement
is blocked while any live stack/TLS lease intersects it, so packed TLS blocks can
share one private RW page. Explicit release ends protection without unmapping.
A dropped token keeps protection in place.

Creation failure releases the unpublished user loan even if kernel-stack cleanup
must retry. After publication, all user CPU execution must end before the loan
is released and terminal observation becomes available. The runtime may reclaim
its user resources after native join, independently of later kernel reaping.
Native threads consume no managed-user-stack slot. Mapping-local wait identity
does not replace a runtime protocol for reusing TCBs or atomics in-place.

This implements a native primitive beneath libc. The rounding fixture is not
complete POSIX floating-point environment inheritance. Signal masks, pthread
cleanup/destructors, detach policy, last-thread POSIX process exit behavior and
complete libc/libc++/Rust std ports remain separate work. Metadata and lease
selection remain linear; no hardware performance, fragmentation, contention or
power result is claimed.
