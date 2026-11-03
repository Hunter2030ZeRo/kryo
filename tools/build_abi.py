#!/usr/bin/env python3
"""Build the isolated hard-float Cryolite user ABI probe."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

from build_user import package_image

ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "targets/x86_64-unknown-cryolite.json"
TARGET_DIR = ROOT / "build/abi"
TOOLCHAIN = ROOT / "targets/cryolite-toolchain.json"


def compiler_versions():
    expected = json.loads(TOOLCHAIN.read_text())
    versions = {
        "rustc": subprocess.check_output(["rustc", "--version", "--verbose"], cwd=ROOT, text=True),
        "clang": subprocess.check_output(["clang", "--version"], text=True),
        "clang++": subprocess.check_output(["clang++", "--version"], text=True),
    }
    if f"commit-hash: {expected['rustc_commit']}\n" not in versions["rustc"]:
        raise RuntimeError("Cryolite target requires the validated Rust revision in targets/cryolite-toolchain.json")
    for compiler in ("clang", "clang++"):
        if versions[compiler].splitlines()[0] != f"clang version {expected['clang_version']}":
            raise RuntimeError(f"{compiler} differs from the validated Cryolite toolchain")
    return versions


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--fetch-dependencies", action="store_true",
                        help="allow Cargo to fetch missing locked Rust sysroot dependencies")
    arguments = parser.parse_args()
    versions = compiler_versions()
    environment = os.environ.copy()
    environment.pop("RUSTFLAGS", None)
    environment["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join([
        "-C", "code-model=small", "-C", "relocation-model=static", "-C", "no-redzone=yes",
        "-Z", "tls-model=local-exec",
    ])
    command = [
        "cargo", "build", "--manifest-path", str(ROOT / "userspace/abi-probe/Cargo.toml"),
        "--target", str(TARGET), "--target-dir", str(TARGET_DIR),
        "-Zjson-target-spec", "-Zbuild-std=core,compiler_builtins",
        "-Zbuild-std-features=compiler-builtins-mem",
        "--profile", "dev" if arguments.profile == "debug" else "release",
    ]
    if not arguments.fetch_dependencies:
        command.append("--offline")
    if (ROOT / "userspace/abi-probe/Cargo.lock").exists():
        command.append("--locked")
    result = subprocess.run(command, cwd=ROOT, env=environment, check=False)
    if result.returncode:
        raise SystemExit(result.returncode)
    artifact = TARGET_DIR / TARGET.stem / arguments.profile / "abi-probe"
    output = arguments.output or ROOT / "build/abi-probe.elf"
    package_image(artifact, output, mapped_headers=True)
    metadata = {
        "scope": "freestanding C/C++/Rust ABI probe; no libc, libc++ or Rust std",
        "packaging": "runtime load bytes exact except ELF64 section-table bookkeeping fields",
        "profile": arguments.profile,
        **versions,
        "toolchain_sha256": hashlib.sha256(TOOLCHAIN.read_bytes()).hexdigest(),
        "target_sha256": hashlib.sha256(TARGET.read_bytes()).hexdigest(),
        "lock_sha256": hashlib.sha256((ROOT / "userspace/abi-probe/Cargo.lock").read_bytes()).hexdigest(),
        "artifact_sha256": hashlib.sha256(artifact.read_bytes()).hexdigest(),
        "image_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
        "image_bytes": output.stat().st_size,
        "command": command,
    }
    output.with_suffix(".build.json").write_text(json.dumps(metadata, indent=2) + "\n")


if __name__ == "__main__":
    main()
