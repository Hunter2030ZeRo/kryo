# Native ABI probe

This isolated freestanding ELF64 executable links independently compiled Clang C,
Clang C++ and Rust objects using the Cryolite user target. It tests mixed
integer/double calls in both directions, the ninth floating argument on the
stack, C variadic floating arguments and mixed integer/double aggregate returns.
It does not link a C or C++ standard library.

The user target selects x86-64-v3 with the SysV AMD64 hard-float convention. It is
separate from the kernel's soft-float `x86_64-unknown-none` target. Existing native
bootstrap programs retain their current target while this ABI is validated.

Entry captures the SysV initial RSP before making a C-convention CALL into Rust.
The fixture requires argv `abi-probe`, `cryolite-v3`, environment `KRYO_ABI=1`,
AT_PHDR, AT_PHENT, AT_PHNUM, AT_PAGESZ, AT_ENTRY and terminating nulls. The probe
reads the actual mapped ELF header through `__ehdr_start` and checks those
auxiliary values against its read-only header/program-header load. Incoming RDI separately identifies an
immutable version-1 startup record: service 50, instance 1, argument 0, and one
role-1 WRITE-only debug capability. The probe validates the record and actual
granted handle. It emits event 90/value 2 before exiting successfully.

Native yield and sleep run with live 256-bit patterns in YMM0 and YMM15. An
AVX2 integer loop keeps YMM15 live without calls. Event 90/value 0 marks its
start; kernel validation must observe an increased preemption count before the
value-1 event. Both markers run inside the assembly block with YMM state live.
Value 2 reports success only after storing and checking the final vector.
These checks cover native entry state, not a general constructor, exception or
language runtime implementation.

The dedicated linker maps the ELF header and program headers read-only, keeps
the TLS initialization template in a read-only load, and emits PT_TLS for native
C `__thread`, C++ POD `thread_local` and Rust `#[thread_local]` objects. These
objects use local-exec TLS with initialized data, zero-filled storage and
64-byte alignment.

The runtime fixture allocates three fixed private BSS slots: one for the main
thread and one for each of two native child threads. It copies PT_TLS initialized
bytes, clears the remaining storage and puts a minimal self-pointer TCB after
the aligned Variant II block before setting FS. Only zero-residue aligned TLS
templates are supported by this fixture. The TCB is sufficient for these
local-exec accesses; it is not mlibc's complete TCB, DTV, canary or cancellation
layout. This is not a general allocation interface.

Each language checks initial data, zero-fill, alignment and private values
across yield/sleep. Children join before the main thread clears and checks their
slots; the main thread's values must remain unchanged. Event 94/value 1 marks
main TLS initialization, and value 2 marks completed joins, slot retirement and
isolation checks. The call-free AVX2 preemption interval runs afterward with no
child runnable. POD TLS needs no destructor callbacks; dynamic TLS and TLS
destructors remain unimplemented.

After the child joins, the same process exercises explicit private memory with
native reserve, commit, decommit and release calls. Its account allows 65,536
reserved pages (256 MiB) and 1,024 committed private pages. Reserving the full
virtual allowance must leave the private-RAM charge at zero; a further
reservation must fail. A 512-page commit exceeds the separate shared-memory
object limit. The probe samples the first and last word of every committed page
for zero-fill, writes distinct volatile patterns and checks them across yield
and sleep. It rejects overlap, malformed alignment, overflow, unreserved commit
and interior-base release.

A rejected 513-page commit must preserve existing contents and accounting.
At 512/1,024 pages this is a quota-admission rejection before publication;
it does not exercise backend rollback or quarantine. Decommitting 16 middle
pages refunds credit, and the previously rejected range then commits completely.
Recommitted middle pages must be zero while their neighbors keep their patterns. A second
reservation tests separate live regions and virtual-quota exhaustion. Release
returns both charges, and first-fit address reuse must expose zero-filled RAM.
Finally, eight reserved pages with four committed pages remain live for process
teardown. Event 95/value 1 follows these checks and precedes the mixed-call/YMM
phase. The kernel checks usage 4/8 at that event and zero after the final thread
and process resources retire.
The probe never dereferences an uncommitted range. This validates the explicit
memory contract, not a heap allocator, demand paging or a hosted runtime.
Its per-process limits are 4,096 mappings and 128 tables. Intermediate page
tables remain until process-space close; data/reservation credit checks do not
prove that those table frames have already been reclaimed.

The subsequent private-word phase uses aligned 32-bit atomics in static ELF
data and committed private VM. A changed value returns immediately without a
wait charge. A wake without a waiter is not remembered. An absolute five-ms
timeout must not return before its deadline. Two children register in a known
order, verified through live wait charges, then wake-one returns only the first
and wake-zero leaves the other blocked. A final bounded wake releases it.

Separate children check control cancellation, decommit cancellation, recommit
with a fresh mapping identity, release cancellation and reservation-address
reuse. A final owner publishes its kernel-managed TLS address; a sibling parks
there, and owner exit/cleanup must cancel that sibling before TLS backing is
reused. Results and predicates use Release/Acquire atomics; revocation cases
hold no Rust references into the affected mappings. Raw entry trampolines and
native joins retain child argument/code/data lifetimes.

Event 96/value 1 marks completed word checks and zero remaining wait charges.
The phase creates eight children in groups of at most two; the kernel reaps
ended children between execution batches, retaining the three-thread account
limit. It leaves the earlier four/eight private/reserved pages for process
cleanup. These checks exercise the experimental private park/wake contract,
not shared-memory waits, pthreads, a lock implementation or measured hardware
latency. The timeout result alone does not prove the thread entered the idle
halt path; kernel observation is required for that claim.

Build with `python3 tools/build_abi.py --profile debug|release`. The script builds
core and compiler builtins for the custom target in `build/abi`, using the
installed nightly rust-src and Clang. It packages with the existing ELF entry/
load-data preservation check and records compiler revisions, target/lock/image
hashes and the build command beside the packaged ELF as `.build.json`.

The default build is offline. On a new Rust source installation, add
`--fetch-dependencies` once to permit fetching its locked sysroot dependencies.

ELF is the binary file format; SysV AMD64 defines the function-call convention.
Neither implies Linux syscall compatibility. This probe does not implement libc,
Rust std, libc++ or dynamic linking. Native call version 0 and this
bootstrap target remain experimental. The compiler revisions recorded by the
builder are validation inputs, not an upstream-supported or distributed sysroot.
`targets/cryolite-toolchain.json` pins the validated Rust commit and Clang
version; the builder rejects another revision before compiling this target.
The kernel's rolling toolchain configuration remains independent.
