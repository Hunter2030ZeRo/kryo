# VirtIO block validation driver

This native Ring 3 program implements the VirtIO block request protocol over
Kryo's bounded queue transport. The kernel owns private descriptors and DMA
storage; user requests name retained memory objects and byte offsets.

The full probe reads block zero and requires `CRYOLITE-BLOCK-PROBE-V1` before any
write. On a writable fixture it writes 4096 bytes to sector 8, flushes, reads the
block back and verifies every byte. Byte `i` is `(i * 37 + 11) & 255`. On a
read-only fixture it instead requires a deliberate write to fail with device
status 1. Queue completion is checked independently from the block status.

The program also rejects malformed buffer records, tests an attenuated device
handle, refuses a second in-flight submission and checks stale tickets. After
acceptance it changes the user buffer description to verify that the private
request snapshot still controls completion. Mode 1 exits after submitting a
second read, leaving device reset and request ownership to process retirement.

The image always includes this executable, but the kernel runs the storage probe
only when `make iso BLOCK_PROBE=1` adds its exact opt-in module. Use a disposable
file-backed fixture containing the required prefix; the writable probe changes
block 1. Default images do not enable this probe.

This is a bootstrap validation driver, not a filesystem or a persistent block
service. There is no BetAFS parser or mount implementation here.
