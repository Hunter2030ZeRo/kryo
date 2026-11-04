# Native C service calls

Date: 2026-11-04. Status: implemented SDK extension.

Expose existing IPC, capability, notification and shared-memory calls in the
freestanding C11/C++23 header. Preserve the native call numbers and register
contract; no kernel mechanism, POSIX descriptor or service protocol is added.

Four-word messages and at most two grants use fixed registers. Inline assembly
declares every input/output register and the compiler memory/condition-code
barriers. It preserves the language ABI even with optimization disabled, PIC
and a frame pointer. Wrapper objects require no libc helper symbols.

Signed status stays separate from full-width values. Receive exposes message,
reply token and installed handles. Existing Call exposes an inline reply only;
its raw handle/count fields are not receive results. Failure delivery state
remains explicit so an accepted timeout cannot be mistaken for a cancelled
shared-memory loan. C aggregates describe registers, not Rust memory layouts.

The optional ABI fixture connects a C client to the existing Rust service peer
in a separate process. Bootstrap Receive supplies a SEND-only endpoint. The
client checks attenuated authority and rejected transfers before sending a
single READ|MAP memory grant. The peer rejects writable mapping, checks all
512 payload words, unmaps and closes before the reply. A read-only recipient
does not make other aliases immutable; the fixture prohibits sender writes
through that interval.

The client closes its memory owners after the acknowledged loan, sends STOP,
observes native process completion and checks endpoint owner death. Native
completion and actual process reaping remain distinct. The kernel requires
fourteen retired threads across both processes, zero surviving channels,
objects/waits/quota, and balanced logical heap and physical backing.

Header validation covers layout, C/C++ O0/O2, PIC/frame-pointer builds and
undefined-symbol checks. Guarded host checks execute rejected-input paths only
after disassembly excludes int80. Guest behavior and source hashes are recorded
in the [validation record](../validation/2026-11-04-c-ipc/README.md).

Reply capabilities and their receive reservations are a separate additive ABI
extension. File open descriptions, standard streams and PTYs still need their
own authority and lifetime contracts; DebugWrite remains diagnostic only.
