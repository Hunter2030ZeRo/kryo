# Native smoke program

This standalone `no_std` Rust executable uses experimental native calls rather
than the Linux or NT ABI. It has no dependencies and is not a kernel workspace
member. Build it with `make user` or `python3 tools/build_user.py`.

The image is a static x86-64 ELF at `0x400000` with separate RX, RO and RW load
segments. Rust checks initialized data, volatile reads from RO data, zeroed BSS
and an instance-specific BSS checksum. Entry receives instance 41 or 42 in RDI,
a private TLS page in RSI, feature bits in RDX and a 16-byte-aligned stack.
Call 19 sets FS to that page; the program owns its FS-relative sentinel. An
invalid noncanonical base must return InvalidArgument and preserve an existing
TLS sentinel.

The built-in `x86_64-unknown-none` target keeps Rust scalar and uses soft float.
The build tool replaces inherited Rust flags with small-code-model static user
flags. An assembly probe owns XMM15 and one x87 stack value across two native
yields and two bounded busy loops. It also checks distinct x87 control words,
MXCSR values and TLS contents. When bootstrap feature bit 0 enables AVX, it also
checks an instance-specific YMM14 upper half using AVX1 instructions. The fallback
path executes no AVX instructions. It balances the x87 stack and restores its
caller's control state before returning to Rust. The kernel test must separately require
actual timer preemption of each instance; loop completion alone does not prove it.

`native_smoke_result` records the final probe/BSS checksum before Exit returns
the instance ID. Failure exits use 100..107 or 250 for a Rust panic.

`make iso` always builds and packages `/boot/native-smoke.elf` because the Limine
configuration lists that module with command line `kryo-native-smoke`. A normal
kernel boot may leave it unused; `boot-tests` validates and executes it. This is
a runtime fixture, not a stable user runtime or general service loader.
