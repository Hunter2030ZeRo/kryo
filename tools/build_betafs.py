#!/usr/bin/env python3
"""Cross-build the independent BetAFS engine and its actual codecs for Cryolite."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib

from betafs_native import copy_engine, git, guard_output, native_environment, owned_directory, prepare_openzl, snapshot, verify_registry
from check_betafs import check_output_links
from build_abi import compiler_versions
from build_user import package_image
from std_overlay import ROOT, digest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--betafs-source", type=Path, required=True)
    parser.add_argument("--profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--build-dir", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--source-cache", type=Path,
                        default=Path(os.environ.get("CRYOLITE_LIBC_CACHE", ROOT / "build/libc/cache")))
    parser.add_argument("--manifest-path", type=Path)
    parser.add_argument("--runtime-source", type=Path, default=ROOT)
    parser.add_argument("--jobs", type=int, default=8)
    arguments = parser.parse_args()
    source = arguments.betafs_source.resolve(strict=True)
    if not (source / "crates/betafs-core/Cargo.toml").is_file():
        parser.error("Expected the independent BetAFS workspace")
    output = guard_output(arguments.output or ROOT / "build/betafs-probe.elf", source)
    build = (arguments.build_dir or ROOT / "build/betafs" / arguments.profile).resolve()
    if build.is_relative_to(source) or source.is_relative_to(build):
        parser.error("Build output must be separate from the independent engine")
    check_output_links(build)
    owned_directory(build, "kryo-native-betafs-build-v1\n")
    before = snapshot(source)
    before_status = git(source, "status", "--porcelain=v1")
    engine = build / "engine"
    copy_engine(source, engine, before)
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    registry_choices = sorted((cargo_home / "registry/src").glob("*/rust-openzl-sys-0.1.0"))
    if len(registry_choices) != 1:
        raise RuntimeError("The pinned rust-openzl-sys source must be cached uniquely")
    original_lock = tomllib.loads((engine / "Cargo.lock").read_text())["package"]
    patched_origins = [package for package in original_lock
                       if package["name"] in ("rust-openzl-sys", "libc") and package.get("source")]
    if {package["name"] for package in patched_origins} != {"rust-openzl-sys", "libc"}:
        raise RuntimeError("The original dependency lock must pin the two patched registry sources")
    if {package["name"]: package["version"] for package in patched_origins} != {
        "rust-openzl-sys": "0.1.0", "libc": "0.2.191"
    }:
        raise RuntimeError("The native recipes require rust-openzl-sys0.1.0 and libc0.2.191")
    verified_origins = verify_registry(patched_origins, registry_choices[0].parent)
    recipe = prepare_openzl(registry_choices[0].parent, build / "openzl")
    runtime = build / "runtime"
    runtime_source = arguments.runtime_source.resolve(strict=True)
    subprocess.run(["python3", str(runtime_source / "tools/build_std.py"), "--profile", arguments.profile,
                    "--build-dir", str(runtime), "--source-cache", str(arguments.source_cache.resolve()),
                    "--output", str(build / "runtime-probe.elf"), "--jobs", str(arguments.jobs)],
                   cwd=runtime_source, check=True)
    project = build / "project"
    project.mkdir(exist_ok=True)
    program = arguments.manifest_path.resolve() if arguments.manifest_path else ROOT / "userspace/filesystems/betafs/native-check/Cargo.toml"
    selected = tomllib.loads(program.read_text())
    package = selected["package"]["name"]
    main_path = program.parent / "src/main.rs"
    # Keep sources in their original tree so module paths and relative includes retain meaning.
    quote = lambda path: json.dumps(str(path), ensure_ascii=False)
    manifest = ["[package]", f"name = {quote(package)}", 'version = "0.1.0"',
                'edition = "2024"', 'publish = false', "", "[workspace]", 'resolver = "3"', "",
                "[[bin]]", f"name = {quote(package)}", f"path = {quote(main_path)}", "",
                "[dependencies]",
                *[f'{name} = {{ path = {quote(engine / "crates" / name)} }}'
                  for name in ("betafs-block", "betafs-core", "betafs-tree", "betafs-codec")],
                f'cryolite-allocator = {{ path = {quote(ROOT / "userspace/libc/allocator")}, features = ["std"] }}',
                f'kryo-libsys = {{ path = {quote(ROOT / "userspace/libsys")} }}',
                f'kryo-interfaces = {{ path = {quote(ROOT / "interfaces")} }}', "",
                "[patch.crates-io]", f'libc = {{ path = {quote(runtime / "rust/libc")} }}',
                f'rust-openzl-sys = {{ path = {quote(build / "openzl")} }}', "",
                "[profile.dev]", 'panic = "abort"', 'opt-level = 1', 'debug = true', "",
                "[profile.release]", 'panic = "abort"', 'lto = "thin"', 'codegen-units = 1',
                'opt-level = 3', 'debug = 1', ""]
    (project / "Cargo.toml").write_text("\n".join(manifest))
    if not (project / "Cargo.lock").is_file():
        (project / "Cargo.lock").write_bytes((engine / "Cargo.lock").read_bytes())
    provider = runtime / "provider"
    target = ROOT / "targets/x86_64-unknown-cryolite-std.json"
    environment = native_environment(build, provider, target, runtime / "rust/library", package)
    environment["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join([
        "-C", "relocation-model=static", "-C", "code-model=small", "-C", "no-redzone=yes",
        "-Z", "tls-model=local-exec", "-C", "link-arg=--static", "-C", "link-arg=--gc-sections",
        "-C", "link-arg=--build-id=none", "-C", "link-arg=-zmax-page-size=0x1000",
        "-C", f"link-arg=-T{ROOT / 'userspace/filesystems/betafs/linker.ld'}",
        "-C", f"link-arg={provider / 'meson/sysdeps/cryolite/crt1.o'}",
        "-C", "link-arg=--start-group", "-C", f"link-arg={provider / 'sysroot/usr/lib/libc.a'}",
        "-C", "link-arg=--end-group",
    ])
    program_inputs = {p.relative_to(program.parent).as_posix(): digest(p)
                      for p in [program, *sorted((program.parent / "src").rglob("*"))] if p.is_file()}
    with (build / "resolved-metadata.json").open("w") as stream:
        subprocess.run(["cargo", "metadata", "--manifest-path", str(project / "Cargo.toml"),
                        "--offline", "--format-version", "1"], cwd=ROOT, env=environment,
                       stdout=stream, check=True)
    resolved = tomllib.loads((project / "Cargo.lock").read_text())["package"]
    permitted = []
    for lock in [engine / "Cargo.lock", ROOT / "Cargo.lock", ROOT / "userspace/libc/allocator/Cargo.lock",
                 runtime / "rust/library/Cargo.lock"]:
        permitted.extend(tomllib.loads(lock.read_text())["package"])
    identity = lambda package: (package["name"], package["version"], package.get("source"), package.get("checksum"))
    allowed = {identity(package) for package in permitted if package.get("source")}
    if any(identity(package) not in allowed for package in resolved if package.get("source")):
        raise RuntimeError("Native resolver drifted from the pinned engine/runtime dependency locks")
    registry = verify_registry(resolved, registry_choices[0].parent)
    command = ["cargo", "build", "--manifest-path", str(project / "Cargo.toml"),
               "--target", str(target), "--target-dir", str(build / "cargo"),
               "-Zjson-target-spec", "-Zbuild-std=std,panic_abort",
               "-Zbuild-std-features=compiler-builtins-mem", "--locked", "--offline", "--jobs", str(arguments.jobs),
               "--profile", "dev" if arguments.profile == "debug" else "release"]
    subprocess.run(command, cwd=ROOT, env=environment, check=True)
    artifact = build / "cargo" / target.stem / arguments.profile / package
    assembly = subprocess.check_output(["objdump", "-d", "-M", "intel", str(artifact)], text=True)
    if re.search(r"\b(?:mm[0-7]|emms|femms)\b", assembly):
        raise RuntimeError("Native BetAFS image contains MMX instructions")
    output = guard_output(output, source)
    packed = build / "packaged.elf"
    package_image(artifact, packed, mapped_headers=True, maximum_bytes=8 << 20)
    if before != snapshot(source) or before_status != git(source, "status", "--porcelain=v1"):
        raise RuntimeError("Independent BetAFS sources changed during the native build")
    if program_inputs != {p.relative_to(program.parent).as_posix(): digest(p)
                          for p in [program, *sorted((program.parent / "src").rglob("*"))] if p.is_file()}:
        raise RuntimeError("Native program sources changed during the build")
    for name, expected in recipe["upstream_files"].items():
        if name not in ("build.rs", "vendor/openzl/build/cmake/openzl-deps.cmake"):
            if digest(build / "openzl" / name) != expected:
                raise RuntimeError(f"OpenZL algorithm source changed in the native build: {name}")
    metadata = {
        "scope": "Native Cryolite BetAFS engine with actual Raw/Zstd/Lz4/OpenZL codecs; compilation is not guest execution or storage persistence proof",
        "profile": arguments.profile, "betafs_head": git(source, "rev-parse", "HEAD").strip(),
        "betafs_dirty_status": before_status, "betafs_source_files": before,
        "betafs_unchanged": True, "openzl_recipe": recipe,
        "openzl_algorithm_sources_unchanged": True,
        "program_manifest": str(program), "program_sources": program_inputs,
        "registry_sources": registry,
        "patched_registry_origins": verified_origins,
        "native_interface_sources": {
            p.relative_to(ROOT).as_posix(): digest(p)
            for directory in [ROOT / "interfaces", ROOT / "userspace/libsys"]
            for p in sorted(directory.rglob("*")) if p.is_file() and p.suffix in (".rs", ".toml")},
        "tool_sources": {name: digest(ROOT / "tools" / name)
                         for name in ("build_betafs.py", "betafs_native.py")},
        "target_sha256": digest(target), "resolved_lock_sha256": digest(project / "Cargo.lock"),
        "runtime": json.loads((build / "runtime-probe.build.json").read_text()),
        "runtime_source": str(runtime_source),
        "native_toolchain_sha256": digest(build / "native-toolchain.cmake"),
        "native_rustc_wrapper_sha256": digest(build / "native-rustc.py"),
        "linker_sha256": digest(ROOT / "userspace/filesystems/betafs/linker.ld"),
        "image_sha256": digest(packed), "image_bytes": packed.stat().st_size,
        "codegen": {"cpu": "x86-64-v3", "mmx": False, "panic": "abort",
                    "image_base": 0x10000000, "packaged_maximum_bytes": 8 << 20,
                    "mmx_disassembly_check": True},
        **compiler_versions(),
    }
    manifest = build / "packaged.build.json"
    manifest.write_text(json.dumps(metadata, indent=2, ensure_ascii=False) + "\n")
    output = guard_output(output, source)
    output.parent.mkdir(parents=True, exist_ok=True)
    os.replace(packed, output)
    os.replace(manifest, output.with_suffix(".build.json"))


if __name__ == "__main__":
    main()
