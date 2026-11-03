# Explicit private virtual memory

Date: 2026-11-03. Contract: [ADR 0026](../../decisions/0026-private-vm-and-atomic-waits.md).

The experimental native ABI provides reserve, commit, decommit and release.
Reserved addresses have no data frames. Successful commit supplies zeroed RW/NX
RAM, with separate account charges for virtual reservations and private pages.
The user runtime remains responsible for allocation policy and object lifetimes.

## Validation

All six QEMU TCG configurations passed with 512 MiB guest RAM:

| Profile | CPU model / processors | Mode | Optional modules |
| --- | --- | --- | --- |
| debug | Haswell-noTSX / 1 | boot-tests | ABI probe |
| release | Haswell-noTSX / 1 | boot-tests | ABI probe |
| debug | max / 4 | normal | ABI probe, writable block fixture |
| release | max / 4 | normal | ABI probe, read-only block fixture |
| debug | max / 4 | boot-tests | legacy modules |
| release | max / 4 | normal | legacy modules |

The four ABI runs exercise the actual Ring 3 native calls. They reserve 256 MiB
without a private-page charge, commit 512 pages, sample zero-fill at both ends
of every page, and preserve volatile page patterns across yield and sleep.
They reject malformed, overlapping and out-of-reservation operations, exhaust
the private/virtual quotas, decommit a middle range, reuse refunded credit,
recommit zeroed backing, keep two reservations live and reuse a released base.
The rejected 513-page commit fails quota admission before mapping; it does not
exercise backend rollback.

The probe leaves four committed pages in an eight-page reservation alive at
event 95/1. The kernel checks those live charges, then requires both counters to
reach zero after all three native threads and the process address space retire.
Frame accounting also checks the prior free count, allowing only measured
retained kernel alias-table/metadata growth. Existing SysV startup, static TLS,
C/C++/Rust calls and live YMM preemption proofs remain required. The probe's VM
calls run after child joins; concurrent sibling VM mutation is not a guest test.

All 428 Rust host tests passed: core 329, HAL 77, interfaces 7, libsys 4 and
services 11. Eighteen Python tests passed. Formatting, normal/all-feature kernel
Clippy, changed core model tests, HAL host tests, the userspace workspace and
the custom-target ABI probe passed Clippy with warnings denied. See
[host-validation.json](host-validation.json) for commands and source hashes.

Core models separately inject physical allocation, map publication, unmap and
alias-restoration failures. They check complete rollback, retained unpublished
owners, partial refunds, retry, foreign address-space rejection, lifetime IDs
and unclosed-manager accounting. A null allocator tests reservation, commit
interval and mapping metadata failures with successful retries. These model
tests provide failure evidence; the QEMU probe does not inject incomplete
retirement or verify the process-quarantine branch dynamically.

[evidence.json](evidence.json) records the six runs, packaged ELF/kernel hashes,
block-fixture hashes and scope. Raw logs retain emulator output. The runner
stops its own emulator after required markers; this does not prove guest exit.
[source-manifest.json](source-manifest.json) identifies source/configuration and
runner inputs, excluding documentation and generated files. Initial Clippy
failures are preserved as `prior-lint.log` and `prior-probe-lint.log`; the final
logs use corrected, frozen sources.

## Limits

There is no user allocator, demand paging, anonymous executable memory,
file-backed mapping, atomic park/wake or libc/std integration in this milestone.
Private-page charges cover this explicit API; ELF, managed stacks, legacy TLS
and kernel metadata bytes are outside that counter. Intermediate user tables
remain until full address-space close and can exhaust the per-process table
limit after sparse mapping cycles.

The BSP changes mappings only after all user CPU leases end. This preserves
the current serialized kernel policy. Sorted mapping records provide binary
lookup and bulk removal, but interior insertion still shifts the suffix per
page. Neither this structure nor TCG success establishes hardware latency,
allocator throughput, fragmentation efficiency, scalability or power use.
