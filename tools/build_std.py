#!/usr/bin/env python3
"""Build the experimental Cryolite std profile without changing the installed sysroot."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess

from build_abi import compiler_versions
from build_user import package_image
from std_overlay import PORT, ROOT, digest, prepare


def run(command, **kwargs):
    subprocess.run([str(arg) for arg in command], cwd=ROOT, check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--build-dir", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--source-cache", type=Path,
                        default=Path(os.environ.get("CRYOLITE_LIBC_CACHE", ROOT / "build/libc/cache")))
    parser.add_argument("--libc-source", type=Path,
                        help="unmodified cached libc-0.2.191 crate directory")
    parser.add_argument("--jobs", type=int, default=8)
    arguments = parser.parse_args()
    versions = compiler_versions()
    pins = json.loads((PORT / "sources.json").read_text())
    versions["cargo"] = subprocess.check_output(["cargo", "--version", "--verbose"], text=True)
    if f"commit-hash: {pins['cargo_commit']}\n" not in versions["cargo"]:
        raise RuntimeError("The private build-std source override requires the pinned Cargo revision")
    build = (arguments.build_dir or ROOT / "build/rust-std" / arguments.profile).resolve()
    sysroot = Path(subprocess.check_output(["rustc", "--print", "sysroot"], cwd=ROOT, text=True).strip())
    source = sysroot / "lib/rustlib/src/rust/library"
    libc = arguments.libc_source
    if libc is None:
        cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
        choices = sorted((cargo_home / "registry/src").glob(f"*/libc-{pins['libc_version']}"))
        if len(choices) != 1:
            raise RuntimeError("Pass --libc-source with the pinned unmodified libc crate")
        libc = choices[0]
    library = prepare(build, source, libc.resolve())
    provider = build / "provider"
    run(["python3", ROOT / "tools/build_libc.py", "--profile", arguments.profile,
         "--build-dir", provider, "--source-cache", arguments.source_cache.resolve(),
         "--jobs", arguments.jobs, "--output", provider / "libc-probe.elf"])
    target = ROOT / "targets/x86_64-unknown-cryolite-std.json"
    probe = PORT / "probe"
    environment = os.environ.copy()
    environment.pop("RUSTFLAGS", None)
    environment["__CARGO_TESTS_ONLY_SRC_ROOT"] = str(library)
    environment["CRYOLITE_STD_PROVIDER"] = str(provider)
    environment["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join([
        "-C", "relocation-model=static", "-C", "code-model=small", "-C", "no-redzone=yes",
        "-Z", "tls-model=local-exec",
        "-C", "link-arg=--static",
        "-C", "link-arg=--gc-sections", "-C", "link-arg=--build-id=none",
        "-C", "link-arg=-zmax-page-size=0x1000",
        "-C", f"link-arg=-T{ROOT / 'userspace/libc/probe/linker.ld'}",
        "-C", f"link-arg={provider / 'meson/sysdeps/cryolite/crt1.o'}",
        "-C", "link-arg=--start-group",
        "-C", f"link-arg={provider / 'sysroot/usr/lib/libc.a'}",
        "-C", "link-arg=--end-group",
    ])
    run(["cargo", "build", "--manifest-path", probe / "Cargo.toml", "--target", target,
         "--target-dir", build / "cargo", "-Zjson-target-spec", "-Zbuild-std=std,panic_abort",
         "-Zbuild-std-features=compiler-builtins-mem", "--profile",
         "dev" if arguments.profile == "debug" else "release", "--locked", "--offline"], env=environment)
    artifact = build / "cargo" / target.stem / arguments.profile / "rust-std-probe"
    assembly = subprocess.check_output(["objdump", "-d", "-M", "intel", str(artifact)], text=True)
    if re.search(r"\b(?:mm[0-7]|emms|femms)\b", assembly):
        raise RuntimeError("Native Rust std probe contains MMX instructions")
    output = (arguments.output or ROOT / "build/rust-std-probe.elf").resolve()
    package_image(artifact, output, mapped_headers=True)
    metadata = {
        "scope": "experimental restricted std: allocation, threads, futex synchronization, TLS, monotonic time and basic service-backed file I/O and optional CPU random output; no wall clock, metadata, positioned I/O, durability, network or process spawning",
        "profile": arguments.profile, **versions,
        "rust_sources": pins, "target_sha256": digest(target),
        "build_tool_sha256": digest(Path(__file__)),
        "codegen": {"cpu": "x86-64-v3", "mmx": False, "panic": "abort"},
        "random_source": {"kind": "CPU RDRAND", "trust": "CPU-only bootstrap",
                          "native_result_bytes": 16, "bridge_atomic_max": 256,
                          "entropy_quality_validated": False},
        "overlay_inputs_sha256": (build / "rust/overlay-inputs.sha256").read_text().strip(),
        "provider": json.loads((provider / "libc-probe.build.json").read_text()),
        "probe_sources": {p.relative_to(probe).as_posix(): digest(p)
                          for p in sorted(probe.rglob("*")) if p.is_file()},
        "image_sha256": digest(output), "image_bytes": output.stat().st_size,
    }
    output.with_suffix(".build.json").write_text(json.dumps(metadata, indent=2) + "\n")


if __name__ == "__main__":
    main()
