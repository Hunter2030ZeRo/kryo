# Packed kernel heap validation

Date: 2026-11-04. Scope: upstream Talc over Kryo-owned page backing, ordinary
allocation failure, physical/logical accounting and concurrent allocation.

## Guest results

| QEMU TCG configuration | Result |
| --- | --- |
| Haswell-noTSX, 1 CPU, debug and release, native ABI boot tests | Passed |
| max, 4 CPUs, debug and release, native ABI and disposable RW/RO block boot tests | Passed |
| max, 4 CPUs, debug and release, normal supervisor/native ABI/RW/RO block startup | Passed |
| max, 65 CPUs, 129 GiB, release boot tests | Passed |

[Evidence](evidence.json) records all seven cases. Boot tests require both
packing and exact all-CPU retirement markers; USERDIAG cannot forge them.
Normal boot omits these test probes while exercising the same global allocator.

The packing fixture holds 128 64-byte objects live, verifies their contents,
then frees them. It observes eight additional backing pages and two pre-existing
metadata pages. The prior per-object algorithm requires 128 pages by its own
page calculation. This is a synthetic memory-footprint comparison, not a
measured speedup. Metadata is explicitly retained rather than reported as free.

The concurrent probe includes BSP and every online AP. Per-round barriers keep
all varied-size/alignment allocations live through unique-word verification.
Only after every AP acknowledges completion does BSP require exact backing,
live allocation count and requested-byte restoration. The 65-CPU case also
verifies retirement across a CPU-set word boundary and high physical addresses.

Physical checks account for shared heap backing once. Separate logical checks
exclude only known retained kernel-table Vec storage; crediting heap pages
cannot hide transient object leaks. Actual task/process/resource retirement
checks remain required.

## Compiler and host verification

The installed nightly changed from the old native ABI pin. The guard rejected
the first ABI build before guest execution. The new pin is rustc commit
708c94ef093ecc42fb8851e17d8383d999727abc; Clang remains 23.1.4. Version checks
remain enabled. The full mixed-language ABI runs above revalidate this compiler.

Rust host suites pass 453 cases; extracted diagnostic tests pass seven more.
Header checks pass 32 cases and Python runner tests pass 25. Formatting,
all-feature kernel Clippy and custom-target ABI Clippy pass with warnings denied.

After application, 1,403 actual-checkout file hashes matched the reviewed
snapshot. The actual source passed header/Python/format checks, normal and
all-feature kernel builds and debug/release native ABI builds. Generated outputs
were relocated. Both linked native probes contain no MMX instructions. The QEMU
matrix runs over identical staged source; actual source rebuilding is separate
evidence. No real-hardware timing, power, kernel SOF or fully parallel kernel
claim follows from these results.

## Preserved initial failures

`prior-toolchain/` records the expected compiler-pin rejection. `prior-wide/`
records an exact FrameStats assertion that failed after the heap changed its
backing lifetime. That assertion ran while task/object containers were alive
and did not account for retained kernel storage. The final check explicitly
drops those containers, requires logical usage equality and balances physical
frames including retained storage. The final wide run reports retained kernel
storage changing from three to five pages and passes complete task teardown.

## Reproduction

```sh
rtk proxy python3 tools/boot_smoke.py --all --cpu Haswell-noTSX --abi-probe
rtk proxy python3 tools/boot_smoke.py --profile debug --cpu max --cpus 4 --abi-probe --block-probe
rtk proxy python3 tools/boot_smoke.py --profile release --cpu max --cpus 4 --abi-probe --block-probe --block-readonly
rtk proxy python3 tools/boot_smoke.py --profile debug --cpu max --cpus 4 --normal --abi-probe --block-probe
rtk proxy python3 tools/boot_smoke.py --profile release --cpu max --cpus 4 --normal --abi-probe --block-probe --block-readonly
rtk proxy python3 tools/boot_smoke.py --profile release --cpu max --cpus 65 --memory 129G
```

Build guest cases serially in an isolated checkout. Raw absolute paths identify
the original verification workspace. Block writes target runner-owned disposable
images only. Host stopping of QEMU does not establish guest shutdown.
