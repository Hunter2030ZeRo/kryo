#!/usr/bin/env python3
"""Build a native ELF with isolated user-code flags."""

import argparse
import os
from pathlib import Path
import shutil
import struct
import subprocess


ROOT = Path(__file__).resolve().parents[1]
PROGRAMS = (
    "native-smoke", "init", "bootstrap-worker", "device-probe", "scale-probe",
    "service-probe", "service-peer", "supervisor", "echo-service", "registry-client",
    "virtio-block", "block-service", "block-client",
)
TARGET = "x86_64-unknown-none"
TARGET_DIR = ROOT / "build" / "userspace"


def load_image(path):
    data = path.read_bytes()
    if len(data) < 64 or data[:6] != b"\x7fELF\x02\x01":
        raise ValueError(f"Expected a little-endian ELF64 image: {path}")
    entry, offset = struct.unpack_from("<QQ", data, 24)
    size, count = struct.unpack_from("<HH", data, 54)
    if size != 56 or offset + size * count > len(data):
        raise ValueError(f"Invalid ELF program headers: {path}")
    segments = []
    for index in range(count):
        kind, flags, start, virtual, physical, file_size, memory_size, alignment = struct.unpack_from(
            "<IIQQQQQQ", data, offset + index * size)
        if kind != 1:
            continue
        if start + file_size > len(data) or file_size > memory_size:
            raise ValueError(f"Invalid ELF load segment: {path}")
        segments.append((flags, virtual, physical, file_size, memory_size, alignment,
                         data[start:start + file_size]))
    return entry, segments


def package_image(artifact, output):
    objcopy = shutil.which("llvm-objcopy") or shutil.which("objcopy")
    if objcopy is None:
        raise RuntimeError("Boot image packaging requires llvm-objcopy or objcopy")
    if artifact.resolve() == output.resolve():
        raise ValueError("Boot output must differ from the symbolful Cargo artifact")
    output.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([objcopy, "--strip-debug", str(artifact), str(output)], check=True)
    if load_image(artifact) != load_image(output):
        raise RuntimeError(f"Debug stripping changed executable load data: {output}")
    size = output.stat().st_size
    if size > 1 << 20:
        raise ValueError(f"Boot module exceeds the one-MiB limit: {output} ({size} bytes)")
    print(f"{output}: {artifact.stat().st_size} -> {size} bytes, entry/load data preserved; symbols: {artifact}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--program", choices=PROGRAMS, default="native-smoke")
    parser.add_argument("--output", type=Path)
    arguments = parser.parse_args()
    package = ROOT / "userspace" / arguments.program
    output = arguments.output or ROOT / "build" / f"{arguments.program}.elf"
    environment = os.environ.copy()
    environment.pop("RUSTFLAGS", None)
    environment["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join([
        "-C", "code-model=small",
        "-C", "relocation-model=static",
        "-C", "no-redzone=yes",
    ])
    command = [
        "cargo", "build", "--manifest-path", str(package / "Cargo.toml"),
        "--package", arguments.program,
        "--target", TARGET, "--target-dir", str(TARGET_DIR),
        "--profile", "dev" if arguments.profile == "debug" else "release",
        "--locked", "--offline",
    ]
    subprocess.run(command, cwd=package, env=environment, check=True)
    artifact = TARGET_DIR / TARGET / arguments.profile / arguments.program
    package_image(artifact, output)


if __name__ == "__main__":
    main()
