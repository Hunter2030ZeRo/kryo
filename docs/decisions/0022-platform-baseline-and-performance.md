# Platform baseline and performance work

Date: 2026-11-04. Status: requirements fixed; runtime and GPU interfaces pending.

## Requirements

Cryolite native C, C++ and Rust programs target x86-64-v3. The minimum includes
the earlier psABI levels and AVX2, BMI1/2, F16C, FMA, LZCNT and MOVBE. AVX
execution also requires OSXSAVE and enabled x87/SSE/YMM state. Hardware admission
is distinct from enabling that execution state and selecting compiler flags.

Every participating CPU must pass the early hardware check. Reject an
unsupported machine before publishing user execution rather than silently
selecting a lower instruction baseline. Early code remains conservatively
compiled. The kernel stays soft-float; applying general user SIMD flags to it
would violate the current entry and state-saving contract. Existing Rust boot
programs still use their freestanding target. A dedicated v3 hard-float native
target now serves the explicitly enabled freestanding mixed-language probe;
the standard-library sysroot remains unimplemented. See
[bootstrap ABI](0024-native-bootstrap-stack.md).

Intel is the first hardware Vulkan target. The observed development machine has
a Meteor Lake-P Arc iGPU, PCI ID 8086:7d55. This read-only host inventory is not
Kryo device execution evidence. Vulkan, general drivers and presentation remain
user-space responsibilities. No GPU driver is implemented by this decision.

## Code generation and legacy boundary

Performance is a primary design requirement. Review IPC transitions and copies,
allocation and state-storage costs, scheduling latency and idle wakeups while
designing an interface, before treating an optimization as later cleanup.
Record the workload and measurements behind a performance claim. A functional
QEMU result or a newer instruction alone does not prove an improvement.

Native production libraries and applications use the v3 baseline directly;
do not add v1/v2 dispatch variants or a lower-baseline platform fallback.
AVX2, BMI1/2, FMA and the other v3 features may be used without per-call feature
checks after CPU admission. Features outside v3, including AVX-512, still need
explicit detection and execution-state support. Conservative early boot and
the kernel's soft-float boundary remain necessary for safe initialization and
entry; user compilation flags do not apply to those paths.

Disable MMX code generation in the native Rust target and C/C++ build flags.
Do not add MMX implementations to new native library paths. The cumulative
v3 hardware checks still include MMX/FPU; their presence does not select a
legacy implementation. General float/double arithmetic uses SSE/AVX. Retain
x87 only where required by the selected 80-bit long double ABI, floating-point
environment and state isolation. Do not change long double precision merely
to remove an instruction family, or enable global fast-math that changes
observable arithmetic behavior.

The native entry path currently eagerly preserves x87/SSE/YMM with standard
XSAVE. Its 1024-byte context buffer is combined state storage, not an x87-only
allocation. Removing a C type would not change that path. Measure save/restore
and scheduling costs separately before selecting state-saving optimizations;
optional instruction support must not silently raise the v3 minimum. Internal
FXSAVE model coverage is not a platform fallback.

## Current costs to measure

| Area | Current implementation | Measurement |
| --- | --- | --- |
| IPC and SMP | CPU batches return ownership before BSP call processing | Same/cross-CPU round-trip distributions; copies, allocations and IPIs per call |
| Scheduling | Round-robin runnable and wait-deadline scans | Wake-to-run latency and throughput as CPU, live and vacant slot counts grow |
| Memory | Page-backed physically contiguous kernel heap allocations | Physical-byte amplification, fragmentation failures and allocation tail latency |
| I/O | Single pending private transport, bounce copies and periodic polling | Throughput and tail latency by queue depth, copied bytes and idle wakeups |

These are structural limitations to evaluate, not measured performance results.
QEMU functional tests do not establish hardware latency, scaling or power use.
Runtime porting and work on these limits must proceed together. Do not expose
current batch coordination, registry indexing or transport descriptors as a
stable language ABI.

## Decisions requiring review

- Choose a libc provider after comparing its native port boundary, pthread/TLS
  behavior, library compatibility, maintenance and measurable costs. mlibc is
  the first evaluation candidate with an explicit sysdeps boundary. This selects
  an evaluation order, not final adoption or a performance ranking. Compare musl
  and relibc using the same criteria. See [the port evaluation](0023-libc-port-evaluation.md).
- Define parallel kernel execution and per-CPU queue ownership before replacing
  BSP coordination. Preserve Parking/wake races, process fault policy,
  capability reservation and acknowledged TLB retirement. Scheduling fairness,
  priority inheritance and CPU-time budgets require explicit policy decisions.
- Evaluate registered bulk submission/completion queues with bounded capacity,
  batched work and notification/IRQ wakeups alongside small control RPCs. Keep
  cancellation separate from completion of a CPU or device loan. Removing the
  private bounce path requires demonstrated safe submission and isolation.
- Compare an ANV native device backend with a DRM-compatible user-space facade.
  Both require actual GPU services; neither means importing Linux kernel policy
  into Kryo or routing native graphics through ICE. GPU VM ownership, memory
  coherency, fences, reset and device isolation must be designed before a public
  GPU submission interface is frozen.

No scheduler replacement, libc selection, stable ABI or GPU interface is
authorized by a proposed alternative in this document. Such choices are reviewed
with the maintainer before implementation.

## Intel Vulkan path

Use existing Vulkan implementations where practical. Mesa ANV is the initial
Intel implementation to evaluate. It expects an operating-system device
backend, and Intel Arc-era devices require GuC firmware for normal operation.
Firmware availability and redistribution must be checked for the selected
revision; GPU initialization and reset need their own implementation evidence.

The native device boundary needs owned buffers, distinct CPU/GPU/DMA addresses,
GPU VM binding, bounded submission, dependencies, completion fences/timelines,
fault handling and reset-before-release. GPU page tables alone do not establish
system DMA isolation. Keep live GPU and display access accounted until actual
retirement, including after process death or timeout.

Build command buffers in user space and evaluate batching at submission
boundaries. Avoid a kernel transition or service RPC for every draw operation.
Presentation additionally needs display/input services, compositor integration
and Vulkan WSI; an offscreen result does not establish desktop readiness.

Verify in stages: native runtime, modeled GPU ownership/failure rules, device
initialization and reset, offscreen compute/render with readback, then
presentation and conformance coverage. Software Vulkan and QEMU Venus are
independent bring-up aids, not evidence of Intel hardware support.

## References

- [x86-64 psABI micro-architecture levels](https://gitlab.com/x86-psABIs/x86-64-ABI/-/blob/master/x86-64-ABI/low-level-sys-info.tex)
- [mlibc port boundary and supported applications](https://github.com/managarm/mlibc/blob/master/README.md)
- [Mesa ANV](https://docs.mesa3d.org/drivers/anv.html)
- [Xe command submission](https://docs.kernel.org/gpu/xe/xe_cs.html)
- [Xe memory management](https://docs.kernel.org/gpu/xe/xe_mm.html)
- [Vulkan loader](https://docs.vulkan.org/guide/latest/loader.html)

The Linux Xe descriptions are implementation references; they do not select a
Linux-compatible Cryolite ABI or prove that Xe can run unchanged on Kryo.
