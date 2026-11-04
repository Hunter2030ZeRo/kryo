#!/usr/bin/env python3
"""Build the pinned static ANSI mlibc bring-up profile and its native C probe."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import urllib.request

from build_abi import compiler_versions
from build_user import package_image

ROOT = Path(__file__).resolve().parents[1]
PORT = ROOT / "userspace/libc/mlibc"
PINS = PORT / "sources.json"
FLAGS = [
    "--target=x86_64-unknown-none", "-march=x86-64-v3", "-mno-mmx",
    "-mno-red-zone", "-D__cryolite__=1", "-ffreestanding", "-fno-stack-protector", "-fno-pic",
    "-fno-pie", "-ftls-model=local-exec", "-ffunction-sections", "-fdata-sections",
    "-fno-unwind-tables", "-fno-asynchronous-unwind-tables",
]


def run(command, **kwargs):
    if "env" not in kwargs:
        environment = os.environ.copy()
        for name in ("CC", "CXX", "AR", "LD", "STRIP", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS"):
            environment.pop(name, None)
        kwargs["env"] = environment
    subprocess.run([str(arg) for arg in command], check=True, cwd=ROOT, **kwargs)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_manifest(source):
    result = {}
    for path in sorted(source.rglob("*")):
        name = path.relative_to(source).as_posix()
        if name == "subprojects/.wraplock":
            continue
        if path.is_symlink():
            result[name] = {"link": os.readlink(path)}
        elif path.is_file():
            result[name] = {"sha256": digest(path), "mode": path.stat().st_mode & 0o777}
    return result


def input_fingerprint(paths):
    entries = [(path.relative_to(ROOT).as_posix(), digest(path))
               for path in paths if path.is_file()]
    return hashlib.sha256(json.dumps(sorted(entries)).encode()).hexdigest()


def archive(cache, name, pin, fetch):
    path = cache / f"{name}-{pin['revision']}.tar.gz"
    if not path.exists():
        if not fetch:
            raise RuntimeError(f"Missing pinned source {path}; use --fetch-dependencies once")
        cache.mkdir(parents=True, exist_ok=True)
        url = pin["repository"].replace("https://github.com/", "https://codeload.github.com/")
        url += "/tar.gz/" + pin["revision"]
        with urllib.request.urlopen(url, timeout=60) as response:
            data = response.read()
        if hashlib.sha256(data).hexdigest() != pin["archive_sha256"]:
            raise RuntimeError(f"Downloaded source hash mismatch: {name}")
        path.write_bytes(data)
    if digest(path) != pin["archive_sha256"]:
        raise RuntimeError(f"Cached source hash mismatch: {path}")
    return path


def extract(path, destination):
    destination.mkdir(parents=True, exist_ok=True)
    with tarfile.open(path) as source:
        members = source.getmembers()
        prefixes = {member.name.split("/")[0] for member in members}
        if len(prefixes) != 1:
            raise RuntimeError(f"Unexpected archive roots: {path}")
        source.extractall(destination.parent, filter="data")
    extracted = destination.parent / prefixes.pop()
    if extracted != destination:
        destination.rmdir()
        extracted.rename(destination)


def prepare(build, cache, fetch):
    pins = json.loads(PINS.read_text())
    inputs = [PINS, Path(__file__), ROOT / "userspace/libsys/include/cryolite/native.h",
              *sorted((ROOT / "userspace/libc/abi/include").rglob("*.h")),
              *sorted((PORT / "sysdeps").rglob("*")),
              *sorted((PORT / "patches").rglob("*"))]
    fingerprint = input_fingerprint(inputs)
    stamp = build / "source-inputs.sha256"
    manifest = build / "source-manifest.json"
    owner = build / "source-owner"
    source = build / "source"
    if source.exists():
        if not owner.is_file() or owner.read_text() != "cryolite-pinned-mlibc-v1\n":
            raise RuntimeError(f"Refusing to replace an unowned source tree: {source}")
        if (stamp.exists() and stamp.read_text().strip() == fingerprint
                and manifest.exists() and json.loads(manifest.read_text()) == source_manifest(source)):
            return source
    # This tree is generated solely from the checked archives and port overlay.
    if source.exists():
        shutil.rmtree(source)
    build.mkdir(parents=True, exist_ok=True)
    owner.write_text("cryolite-pinned-mlibc-v1\n")
    extract(archive(cache, "mlibc", pins["mlibc"], fetch), source)
    for name, pin in pins.items():
        if name != "mlibc":
            extract(archive(cache, name, pin, fetch), source / "subprojects" / name)
    packages = source / "subprojects/packagefiles"
    for name in ("freestanding-c-hdrs", "freestanding-cxx-hdrs"):
        shutil.copytree(packages / name, source / "subprojects" / name, dirs_exist_ok=True)
    run(["patch", "--batch", "-p1", "-d", source / "subprojects/freestanding-cxx-hdrs",
         "-i", packages / "freestanding-cxx-hdrs-cxx23.patch"])
    for patch in sorted((PORT / "patches").glob("*.patch")):
        run(["patch", "--batch", "-p1", "-d", source, "-i", patch])
    shutil.copytree(PORT / "sysdeps/cryolite", source / "sysdeps/cryolite")
    # All public scalar/layout inputs come from the owned Cryolite profile.
    includes = source / "sysdeps/cryolite/include/cryolite"
    shutil.copytree(ROOT / "userspace/libc/abi/include/cryolite", includes, dirs_exist_ok=True)
    shutil.copy2(ROOT / "userspace/libsys/include/cryolite/native.h", includes / "native.h")
    manifest.write_text(json.dumps(source_manifest(source), indent=2) + "\n")
    stamp.write_text(fingerprint + "\n")
    return source


def cross_file(build, profile):
    path = build / "cryolite.cross"
    flags = FLAGS + (["-O2"] if profile == "release" else ["-O1", "-g"])
    path.write_text("\n".join([
        "[binaries]", "c = 'clang'", "cpp = 'clang++'", "ar = 'llvm-ar'", "strip = 'llvm-strip'",
        "[host_machine]", "system = 'cryolite'", "cpu_family = 'x86_64'",
        "cpu = 'x86-64-v3'", "endian = 'little'", "[properties]", "needs_exe_wrapper = true",
        "[built-in options]", f"c_args = {flags!r}", f"cpp_args = {flags!r}",
        "c_link_args = ['--target=x86_64-unknown-none', '-fuse-ld=lld', '-nostdlib']",
        "cpp_link_args = ['--target=x86_64-unknown-none', '-fuse-ld=lld', '-nostdlib']", "",
    ]))
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--build-dir", type=Path)
    parser.add_argument("--source-cache", type=Path,
                        default=Path(os.environ.get("CRYOLITE_LIBC_CACHE", ROOT / "build/libc/cache")))
    parser.add_argument("--fetch-dependencies", action="store_true")
    parser.add_argument("--jobs", type=int, default=8)
    arguments = parser.parse_args()
    versions = compiler_versions()
    build = (arguments.build_dir or ROOT / "build/libc" / arguments.profile).resolve()
    source = prepare(build, arguments.source_cache.resolve(), arguments.fetch_dependencies)
    cross = cross_file(build, arguments.profile)
    configured = build / "meson"
    sysroot = build / "sysroot"
    setup = ["meson", "setup", configured, source, "--cross-file", cross,
             "--prefix=/usr", "--libdir=lib", "--wrap-mode=nodownload",
             "--force-fallback-for=frigg,libsmarter,freestanding-c-hdrs,freestanding-cxx-hdrs",
             "-Ddefault_library=static", "-Db_staticpic=false", "-Dlibgcc_dependency=false",
             "-Dposix_option=disabled", "-Dlinux_option=disabled", "-Dglibc_option=disabled",
             "-Dbsd_option=disabled", "-Duse_freestnd_hdrs=enabled", "-Dbuild_tests=false"]
    if (configured / "build.ninja").exists():
        setup.append("--reconfigure")
    run(setup)
    run(["ninja", "-C", configured, f"-j{arguments.jobs}"])
    if sysroot.exists():
        shutil.rmtree(sysroot)
    shutil.copytree(source / "subprojects/freestanding-c-hdrs/include",
                    sysroot / "usr/include", dirs_exist_ok=True)
    run(["meson", "install", "-C", configured, "--no-rebuild", "--destdir", sysroot])
    # Port-specific ABI views take precedence over the provider's generic views.
    shutil.copytree(source / "sysdeps/cryolite/include", sysroot / "usr/include", dirs_exist_ok=True)
    target = ROOT / "targets/x86_64-unknown-cryolite.json"
    allocator_dir = build / "allocator"
    environment = os.environ.copy()
    environment.pop("RUSTFLAGS", None)
    environment["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join([
        "-C", "relocation-model=static", "-C", "code-model=small", "-C", "no-redzone=yes",
        "-Z", "tls-model=local-exec",
    ])
    run(["cargo", "build", "--manifest-path", ROOT / "userspace/libc/allocator/Cargo.toml",
         "--target", target, "--target-dir", allocator_dir, "-Zjson-target-spec",
         "-Zbuild-std=core,alloc,compiler_builtins", "-Zbuild-std-features=compiler-builtins-mem",
         "--profile", "dev" if arguments.profile == "debug" else "release", "--locked", "--offline"], env=environment)
    allocator = allocator_dir / target.stem / arguments.profile / "libcryolite_allocator.a"
    probe = ROOT / "userspace/libc/probe"
    artifact = build / "libc-probe"
    run(["clang", *FLAGS, "-O2", "-Wall", "-Wextra", "-Werror", "-nostdinc", "-isystem", sysroot / "usr/include",
         "-c", probe / "main.c", "-o", build / "probe.o"])
    for name in ("allocator.c", "allocator_entry.S", "provider_threads.c", "provider_stdio.c"):
        run(["clang", *FLAGS, "-O2", "-fno-builtin", "-nostdinc", "-isystem", sysroot / "usr/include",
             "-Wall", "-Wextra", "-Werror", "-c", probe / name, "-o", build / f"{name}.o"])
    run(["ld.lld", "--static", "--gc-sections", "--build-id=none", "-zmax-page-size=0x1000",
         "-T", probe / "linker.ld", configured / "sysdeps/cryolite/crt1.o", build / "probe.o",
         build / "allocator.c.o", build / "allocator_entry.S.o",
         build / "provider_threads.c.o", build / "provider_stdio.c.o",
         "--start-group", sysroot / "usr/lib/libc.a", allocator, "--end-group", "-o", artifact])
    assembly = subprocess.check_output(["objdump", "-d", "-M", "intel", str(artifact)], text=True)
    symbols = subprocess.check_output(["llvm-nm", str(artifact)], text=True)
    if re.search(r"\b(?:mm[0-7]|emms|femms)\b", assembly):
        raise RuntimeError("Native libc probe contains MMX instructions")
    for function in ("strlen", "strcmp"):
        body = re.search(r"(?ms)^[0-9a-f]+ <" + function + r">:\n(.*?)(?=^[0-9a-f]+ <|\Z)", assembly)
        if not body or (f"__mlibc_{function}_avx2" not in body[1] and "ymm" not in body[1]):
            raise RuntimeError(f"Native libc probe did not link the selected {function} path")
    if re.search(r"__mlibc_(?:strlen|strcmp)_(?:sse2|sse4_2|default)", symbols):
        raise RuntimeError("Native libc probe linked a legacy string fallback")
    output = arguments.output or ROOT / "build/libc-probe.elf"
    package_image(artifact, output, mapped_headers=True)
    metadata = {"scope": "static ANSI mlibc bring-up; no descriptors/pthreads/entropy/hosted ABI release",
                "profile": arguments.profile, "sources": json.loads(PINS.read_text()), **versions,
                "source_inputs_sha256": (build / "source-inputs.sha256").read_text().strip(),
                "toolchain_sha256": digest(ROOT / "targets/cryolite-toolchain.json"),
                "target_sha256": digest(target),
                "allocator_lock_sha256": digest(ROOT / "userspace/libc/allocator/Cargo.lock"),
                "codegen": {"mmx": False, "string_paths": "AVX2 without legacy dispatch"},
                "probe_sources": {p.name: digest(p) for p in sorted(probe.iterdir()) if p.is_file()},
                "allocator_sources": {str(p.relative_to(ROOT)): digest(p)
                    for p in sorted((ROOT / "userspace/libc/allocator").rglob("*"))
                    if p.is_file() and (p.suffix in (".rs", ".toml", ".lock") or p.name == "allocator.h")},
                "image_sha256": digest(output), "image_bytes": output.stat().st_size}
    output.with_suffix(".build.json").write_text(json.dumps(metadata, indent=2) + "\n")


if __name__ == "__main__":
    main()
