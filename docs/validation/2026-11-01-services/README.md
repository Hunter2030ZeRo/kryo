# Native channel creation and service validation

Date: 2026-11-01.

All 12 final QEMU MTTCG configurations pass: default-model one-CPU guests at
512 MiB, plus max-model two-CPU guests at 512 MiB and 129 GiB, each in debug/release
and boot-tests/normal boot. `evidence.json` records commands, checks, timing,
managed frame counts and sampled resident memory. `source-manifest.json` hashes
225 source/configuration/runner inputs. Those inputs stayed unchanged throughout
the final matrix. Markdown, generated builds and validation artifacts are excluded.

Host tests pass 304 cases: core 245, HAL 58, interfaces 1. `host-tests.log` retains
raw output. New models cover shared factory exhaustion, attenuation, rollback,
creator/account destruction and charges retained by delegated or stale references.
`checks.log` records passing normal/all-feature kernel Clippy, scoped core budget
and capability Clippy, userspace workspace Clippy and formatting. The earlier
whole-host-test range.rs identity_op remains outside this clean lint scope.

## Guest checks

The native supervisor receives a ChannelFactory, peer image and debug capability.
The configured eight-channel account belongs to this validation grant; it is not
a global kernel channel limit. The child receives only bootstrap SEND|TRANSFER and
the same factory's CREATE|TRANSFER. Each peer creates its own endpoint and sends
a reduced SEND capability to the supervisor. A real Call/Receive/Reply exchanges
[request, instance, 41, 0] and [response, instance, 42, 0]. The service rejects
RECEIVE delegation, and its client cannot receive using the SEND-only handle.

The supervisor tests notification signal coalescing and timeout, stripped CREATE,
wrong-kind factory access, account exhaustion and reuse. With a peer alive, the
same shared account has only five free charges after bootstrap, notification and
peer endpoint. A second process therefore cannot reset the count. The supervisor
stops each peer, observes its retained exit status and verifies PeerClosed on its
old endpoint. It keeps the first old handle while exchanging with a second peer,
then checks that old authority again. This proves reconnection to a distinct live
object, not a public stable service identity or a wire-level ObjectId. Core and
trusted guest checks separately exercise generational slot reuse.

Boot-only checks force handle, arena and channel-registry capacity failures before
publication, preserving reserved-slot and charge counts. They cover pending waits,
accepted capability references, owner death, registry-only channels and explicit
device notification roots. Settlement preserves accepted grants before cleaning
sender state, resolves stranded waits and removes closed roots outside the arena
callback. Final payload destruction releases the charge; stale external references
keep closed payloads charged.

Both boot modes run the actual Ring 3 service probe, retire all probe processes,
assert zero remaining objects/charges/waits and restore physical-frame counts.
The count allows only the measured growth of retained kernel HHDM split tables
and their metadata. Existing native init/worker supervision then proceeds; its
startup authority is unchanged. Boot-tests also pass the existing isolation,
FP/TLS, shared memory, IPC, device, 127-thread growth and SMP/TLB checks. Large-RAM
guests retain sparse high-physical-address access and accounting checks.

| Configuration | Mode | Managed GiB | Peak RSS MiB | Result |
| --- | --- | --- | --- | --- |
| debug-default-cpu1-512M | boot-tests | 0.46597 | 122.00 | passed |
| release-default-cpu1-512M | boot-tests | 0.47674 | 99.42 | passed |
| debug-default-cpu1-512M | normal | 0.46821 | 103.10 | passed |
| release-default-cpu1-512M | normal | 0.47838 | 82.26 | passed |
| debug-max-cpu2-512M | boot-tests | 0.46548 | 125.04 | passed |
| release-max-cpu2-512M | boot-tests | 0.47649 | 102.46 | passed |
| debug-max-cpu2-512M | normal | 0.46772 | 108.16 | passed |
| release-max-cpu2-512M | normal | 0.47814 | 85.03 | passed |
| debug-max-cpu2-129G | boot-tests | 128.96149 | 149.37 | passed |
| release-max-cpu2-129G | boot-tests | 128.97251 | 126.34 | passed |
| debug-max-cpu2-129G | normal | 128.96373 | 124.17 | passed |
| release-max-cpu2-129G | normal | 128.97415 | 101.21 | passed |

## Scope and reproduction

This is QEMU functional evidence. It does not prove physical hardware behavior,
latency, power, unrestricted service resource isolation, PCI/DMA recovery, a stable
service registry, manifests, ICE, BetAFS, Velocity or KDE integration. General
quotas, process-owned concurrent receivers and consumer migration remain pending.
129-GiB guest RAM is demand-backed and sparsely touched; it is not a complete RAM
write sweep. The runner stops only its own emulator after markers, not through a
guest shutdown path.

Reproduce with `python3 tools/boot_smoke.py --profile debug|release`; add `--normal`
for init mode, `--cpu max --cpus 2` for the SMP selection and `--memory 129G` for
large RAM. Each boot is bounded by the existing 90-second host limit and 2048-MiB
sampled resident-memory guard. Kernel scheduling quanta and user IPC deadlines
were not relaxed to obtain passing results.

`prior-unparked-check.*` preserves the first integration failure. A newly added
trusted helper read a wait completion before performing its Parking-to-Parked
handoff. Adding the missing handoff fixed the helper; it did not alter production
wait, timer or IPC behavior. These artifacts describe intermediate source only.
