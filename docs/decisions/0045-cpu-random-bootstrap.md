# Initial CPU random source

Date: 2026-11-05. Status: CPU-only trust policy accepted; experimental implementation.

Use the CPU hardware random-byte source as the initial sole trusted supplier.
Keep this optional: x86-64-v3 admission does not require RDRAND or RDSEED.
Unsupported hardware and temporary source exhaustion must return distinct errors
without invented bytes. Independent device/firmware sources and later policy
remain follow-ups.

## Mechanism and ABI

Sample RDRAND through HAL while the native dispatcher retains the BSP and user
CPU leases are quiescent. Check the current CPU's CPUID bit and every instruction's
carry flag. A successful zero is valid output. Allow at most ten attempts per
64-bit word; failed fills clear their private sampled prefix. Do not cache a
user CPU's optional feature result across migration to another CPU.

Experimental native call 67, RandomWords, accepts reserved RDI/RSI values zero.
It returns two words in RDX/R8, with the existing status in RAX. Both words are
published only after full success and are zero on failure. Nonzero reserved
inputs are InvalidArgument; no source is Unsupported (-12), and bounded retry
exhaustion is Busy (-11). Existing call and error values remain unchanged.
There is no user pointer, memory mapping, kernel allocation or new authority
object. Like ClockRead, this bounded public mechanism is available to a caller
without a separate capability. Its fixed work bound limits one admission.

The C and Rust SDKs expose the status and two words. A private libc bridge
stages requests of at most 256 bytes from successive register calls, copies to
the caller only after complete success, then clears its temporary storage.
Zero-length requests do not inspect the pointer or source. Errors leave the
destination unchanged; the bridge neither retries failed native calls nor
substitutes fixed data. This is an ordinary C buffer contract in Ring 3, not a
new privileged user-copy path.

The std backend fills bounded chunks through that bridge. Its infallible API
panics on source failure, preserving the profile's panic-abort behavior. Native
and bridge callers retain fallible status. RandomState/HashMap can now obtain
their initial keys on admitted hardware; the rest of the std and hosted ABI
limits remain unchanged.

## Trust and proof boundary

RDRAND supplies hardware DRBG output. Do not describe each returned bit as an
independent sampled entropy bit, or reuse the output with an unsupported claim
of full-entropy seed strength. Intel distinguishes this output from RDSEED seed
material in its [DRNG software guide, revision 2.2](https://cdrdv2-public.intel.com/864722/drng-software-implementation-guide.pdf).
The CPU-only policy is a bootstrap trust choice, not a hardware entropy
assessment or protection against a malicious CPU/hypervisor.

No new software PRNG, cryptographic mixing scheme or entropy estimate is added.
Timestamps, addresses and statistical output differences are not entropy
evidence. The existing libc startup hardening, AT_RANDOM, public getentropy/
getrandom APIs and broader runtime ports remain separate work.

The [validation record](../validation/2026-11-05-cpu-random/README.md) separates
CF/retry and failure-injection tests from QEMU execution. A masked v3 CPU must
still boot while reporting the source unavailable. Future VirtIO RNG support
requires its own device/queue ownership work; see the
[VirtIO entropy-device specification](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html#x1-3240004).
