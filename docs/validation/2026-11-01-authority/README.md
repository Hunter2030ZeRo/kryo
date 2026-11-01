# Startup manifests and service authority validation

Date: 2026-11-01.

All 16 final QEMU MTTCG configurations pass: default-model one-CPU 512-MiB guests,
max-model two- and four-CPU 512-MiB guests, and max-model two-CPU 129-GiB guests,
each in debug/release and boot-tests/normal boot. `evidence.json` retains commands,
checks, managed frames and sampled resident memory. `source-manifest.json` hashes
239 source/configuration/runner inputs, unchanged throughout the final matrix.
Markdown, generated builds and validation artifacts are excluded.

Host tests pass 319 cases: core 246, HAL 58, interfaces 2, libsys 3 and service
policy 10. Raw output is in `host-tests.log`. `checks.log` records passing kernel
normal/all-feature Clippy, scoped core capability Clippy, production userspace
workspace Clippy and formatting. The earlier whole-host-test range.rs identity_op
remains outside that clean lint scope. Independent review approved the corrected
registration lifetime path before application.

## Authority and startup

The initial supervisor receives seven role-labeled capabilities through a READ-only
version-1 startup receipt. Child startup uses four explicit grants, beyond legacy
Spawn's two-grant convention. Kernel fixtures check version/owner/kind/rights,
stale source handles, descriptor mutation and snapshot allocation failure, private
child installation, record-arena exhaustion and actual post-load publication OOM.
Rejected attempts expose no partial child authority; unfinished VM/stack cleanup
remains owned for retirement. Legacy Spawn and two-capability IPC are unchanged.

A real echo service advertises exactly SEND|TRANSFER through its private instance
gate. Client gateways bind authorization to supervisor state, not the client's
claimed identity. The denied client claims the allowed client's ID, requests an
unauthorized service and tries registration through discovery; no grant is issued.
The allowed client requests excess rights and tries RESTART through discovery;
both fail before it completes an ordinary lookup and RPC. Granted client endpoints
are inspected as SEND alone; receive, close/control and direct redelegation fail.

Malformed requests without callbacks, overprivileged callbacks and unsupported
protocol versions do not terminate the broker or acquire authority. A separate
explicit checkpoint capability coordinates bootstrap checks; it is not implicit
lookup or general process-control authority.

A diagnostic service (ID 11, separate scratch inventory) deliberately sends a
CallCaps registration. The supervisor captures every delivered handle before
checking the envelope, rejects the registration, closes the transferred references
and stops the process. It then allocates all eight server-account channels,
observes exhaustion on the ninth and releases them. This checks actual rejected-
registration ownership recovery, including the accepted capability transfer.
The diagnostic never becomes Ready. The ordinary registry then runs service 10
instances 1 and 2, with a fresh admission gate and endpoint for each.

The denied client's eight-channel account stays full while the other client and
server complete their first RPC. Root gates and the three service factories use
distinct accounts. This proves separation of newly created channel charges, not
freedom from all shared-resource interference. Retaining a stale server endpoint
continues to charge that server's account until its last reference retires.

For late-reply handling, the server holds an accepted client reply token while
calling its private checkpoint. The supervisor cancels the known client wait,
requires accepted delivery, then replies to release the server. The old reply
returns InvalidReply and is discarded. The client observes Cancelled/accepted and
completes another RPC on the same instance before replacement. No short deadline
or scheduler-speed assumption is needed; existing accepted-timeout tests run too.
The client retains the first endpoint through replacement, observes PeerClosed,
looks up instance 2 and completes its second-generation RPC.

## Time and retirement

ReceiveFor starts its unchanged one-millisecond polling interval at kernel
admission after kind/right checks. Its fixture covers zero duration, overflow,
wrong type/rights without queue publication, an old absolute deadline rejecting a
queued sender, successful relative receive of that sender and actual timer-driven
expiry of an empty relative receive. Absolute RPC deadlines and scheduling quanta
are unchanged.

Boot-tests require successful root exit, five actual child starts including the
rejected diagnostic, zero remaining objects/waits/account charges and restored
physical frames. Only measured retained HHDM split tables/metadata are exempted.
Normal boot instead observes root readiness and a later heartbeat with the
replacement alive. The monitor's ordinary failed-start backoff exists in source;
that persistent retry branch was not failure-injected in this matrix. Root-supervisor
restart, durable registry epochs and orphan recovery remain unimplemented.

All prior isolation, IPC, FP/TLS, device, legacy init/thread, 127-thread growth and
SMP/TLB checks pass in boot-test mode. Large-memory guests retain sparse high-
physical-address checks. QEMU functional results do not establish physical
hardware behavior, latency, energy use or a complete RAM sweep.

| Configuration | Mode | Peak RSS MiB | Result |
| --- | --- | --- | --- |
| debug-default-cpu1-512M | boot-tests | 120.68 | passed |
| release-default-cpu1-512M | boot-tests | 101.40 | passed |
| debug-default-cpu1-512M | normal | 101.28 | passed |
| release-default-cpu1-512M | normal | 84.38 | passed |
| debug-max-cpu2-512M | boot-tests | 123.36 | passed |
| release-max-cpu2-512M | boot-tests | 102.51 | passed |
| debug-max-cpu2-512M | normal | 102.10 | passed |
| release-max-cpu2-512M | normal | 87.11 | passed |
| debug-max-cpu4-512M | boot-tests | 128.47 | passed |
| release-max-cpu4-512M | boot-tests | 107.28 | passed |
| debug-max-cpu4-512M | normal | 109.23 | passed |
| release-max-cpu4-512M | normal | 92.13 | passed |
| debug-max-cpu2-129G | boot-tests | 147.61 | passed |
| release-max-cpu2-129G | boot-tests | 126.63 | passed |
| debug-max-cpu2-129G | normal | 118.49 | passed |
| release-max-cpu2-129G | normal | 103.36 | passed |

## Packaging and reproduction

`packaging.json` verifies all ten native programs in debug and release. Boot
copies strip debug sections while preserving entry and every PT_LOAD descriptor
and byte; full Cargo debug artifacts stay available. Existing one-MiB module and
eight-load-page limits remain unchanged. The build requires llvm-objcopy or GNU
objcopy and rejects a packaging mismatch.

Run `python3 tools/boot_smoke.py --profile debug|release`, with optional `--normal`,
`--cpu max --cpus 2|4`, and `--memory 129G` for the large two-CPU selection. Each
run stops only its own emulator after markers and retains the existing 90-second
host deadline and 2048-MiB sampled resident-memory guard. This is not guest shutdown
or physical RAM validation. Largest sampled process RSS was below 148 MiB.

## Intermediate evidence

`prior-record-rights.*` records a new trusted fixture describing a broader source
handle with reduced rights; exact receipt validation rejected it before the
intended arena failure. The fixture now uses the source's actual rights.
`prior-module-debug-size.*` records a symbolful ELF exceeding the existing module
byte budget; packaging now strips debug sections and verifies load identity.

`prior-supervisor-*`, `prior-bootstrap-protocol.*` and `prior-expired-admission.*`
retain investigation logs. The initial helper waited for a live child after its
supervisor had already failed; fail-fast root-status and child diagnostics now
surface the actual error. A bounded admission counter then observed 220 expired
absolute receive admissions before queue entry. ReceiveFor fixes that clock-read/
resume gap while retaining the polling duration and absolute RPC budgets.

`prior-partial-matrix.evidence.json` and `prior-matrix-recheck.*` belong to an older
source with a timing-dependent five-millisecond accepted-call assumption. The
final fixture uses the explicit accepted-cancellation handshake described above.
These intermediate records do not attest final-source results.

The configured inventory and binary protocols remain experimental. Dynamic/text
manifests, general memory/handle/wait/CPU budgets, scalable event waits, device/DMA
authority and global descendant revocation remain separate work. Service labels
and protocol fields are not authentication tokens.
