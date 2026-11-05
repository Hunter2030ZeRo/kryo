"""Prepare owned native build inputs without modifying BetAFS or Cargo sources."""

import hashlib
import inspect
import json
import os
from pathlib import Path
import shutil
import shlex
import subprocess
import tarfile

from std_overlay import digest, replace, tree_manifest


def git(root, *arguments):
    return subprocess.check_output(["git", "-C", str(root), *arguments], text=True)


def snapshot(root):
    names = set(git(root, "ls-files", "-z").split("\0"))
    names.update(git(root, "ls-files", "--others", "--exclude-standard", "-z").split("\0"))
    result = {}
    for name in sorted(names):
        if not name:
            continue
        path = root / name
        if path.is_symlink():
            raise RuntimeError(f"External source symlink is not admitted: {path}")
        if path.is_file():
            result[name] = digest(path)
    return result


def owned_directory(path, owner_text):
    owner = path / "source-owner"
    if path.is_symlink() or owner.is_symlink():
        raise RuntimeError(f"Refusing a symlinked output: {path}")
    if path.exists() and (not owner.is_file() or owner.read_text() != owner_text):
        raise RuntimeError(f"Refusing to replace unowned sources: {path}")
    path.mkdir(parents=True, exist_ok=True)
    owner.write_text(owner_text)


def guard_output(path, source):
    path = Path(path).absolute()
    for target in (path, path.with_suffix(".build.json")):
        for component in (target, *target.parents):
            if component.is_symlink():
                raise RuntimeError(f"Native output must not traverse a symlink: {component}")
        resolved = target.resolve()
        if resolved.is_relative_to(source) or source.is_relative_to(resolved):
            raise RuntimeError(f"Native output overlaps the independent BetAFS source: {target}")
        if target.exists() and not target.is_file():
            raise RuntimeError(f"Native output is not a regular file: {target}")
    return path.resolve()


def copy_engine(source, destination, files):
    owned_directory(destination, "kryo-native-betafs-v1\n")
    for path in sorted(destination.rglob("*"), reverse=True):
        if path.is_symlink():
            raise RuntimeError(f"Owned source contains a symlink: {path}")
        if path.is_file() and path.relative_to(destination).as_posix() not in {*files, "source-owner"}:
            path.unlink()
    for name, expected in files.items():
        target = destination / name
        if target.is_symlink():
            raise RuntimeError(f"Owned source contains a symlink: {target}")
        if not target.is_file() or digest(target) != expected:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((source / name).read_bytes())
        if digest(target) != expected:
            raise RuntimeError(f"Source changed while copying: {name}")


def verify_registry(packages, registry):
    result = {}
    cache = registry.parent.parent / "cache" / registry.name
    for package in packages:
        if not package.get("source", "").startswith("registry+"):
            continue
        name = package["name"] + "-" + package["version"]
        archive = cache / (name + ".crate")
        if digest(archive) != package["checksum"]:
            raise RuntimeError(f"Cached crate archive differs from its dependency lock: {name}")
        source = registry / name
        files = {}
        with tarfile.open(archive) as stream:
            for member in stream.getmembers():
                if member.isdir():
                    continue
                if not member.isfile() or not member.name.startswith(name + "/"):
                    raise RuntimeError(f"Unexpected crate archive member: {member.name}")
                relative = member.name[len(name) + 1:]
                if not relative or ".." in Path(relative).parts:
                    raise RuntimeError(f"Invalid crate archive path: {member.name}")
                expected = hashlib.sha256(stream.extractfile(member).read()).hexdigest()
                if digest(source / relative) != expected:
                    raise RuntimeError(f"Registry source differs from its locked archive: {name}/{relative}")
                files[relative] = expected
        actual = tree_manifest(source)
        if set(actual) - set(files) - {".cargo-ok", ".cargo-checksum.json"}:
            raise RuntimeError(f"Registry source has unexpected files: {name}")
        result[name] = {"archive_sha256": package["checksum"],
                        "source_sha256": hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()}
    return result


def prepare_openzl(registry, destination):
    source = registry / "rust-openzl-sys-0.1.0"
    source_files = tree_manifest(source)
    fingerprint = hashlib.sha256(json.dumps(source_files, sort_keys=True).encode()).hexdigest()
    recipe_stamp = fingerprint + ":" + hashlib.sha256(inspect.getsource(prepare_openzl).encode()).hexdigest()
    owner = destination / "source-owner"
    stamp = destination / "upstream.sha256"
    expected = "kryo-native-openzl-v1\n"
    if destination.exists():
        if owner.is_symlink() or not owner.is_file() or owner.read_text() != expected:
            raise RuntimeError(f"Refusing to replace unowned codec sources: {destination}")
        manifest = destination / "native-manifest.json"
        if stamp.is_file() and stamp.read_text().strip() == recipe_stamp and manifest.is_file():
            actual = tree_manifest(destination)
            for name in ("source-owner", "upstream.sha256", "native-manifest.json"):
                actual.pop(name, None)
            if actual == json.loads(manifest.read_text()):
                return {"upstream_tree_sha256": fingerprint, "upstream_files": source_files,
                        "native_build_rs_sha256": digest(destination / "build.rs"),
                        "native_cmake_sha256": digest(destination / "vendor/openzl/build/cmake/openzl-deps.cmake")}
        shutil.rmtree(destination)
    shutil.copytree(source, destination)
    owner.write_text(expected)
    replace(destination / "build.rs", '.define("OPENZL_BUILD_SHARED_LIBS", "OFF")',
            '.define("OPENZL_BUILD_SHARED_LIBS", "OFF")\n'
            '        .define("OPENZL_BUILD_CPP", "OFF")\n'
            '        .define("ZSTD_MULTITHREAD_SUPPORT", "OFF")\n'
            '        .define("ZSTD_BUILD_SHARED", "OFF")\n'
            '        .define("OPENZL_NATIVE_NO_THREADS", "ON")')
    replace(destination / "build.rs", '.define("OPENZL_INSTALL", "OFF");',
            '.define("OPENZL_INSTALL", "OFF")\n        .build_target("openzl");')
    replace(destination / "build.rs", 'if cfg!(target_os = "linux") {',
            'if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {')
    replace(destination / "build.rs", '    let dst = cfg.build();',
            '    let native_flags = env::var("CRYOLITE_NATIVE_CFLAGS").expect("native C flags");\n'
            '    cfg.define("CMAKE_C_FLAGS", &native_flags)\n'
            '        .define("CMAKE_ASM_FLAGS", &native_flags)\n'
            '        .define("CMAKE_CXX_FLAGS", format!("{} -nostdinc++", native_flags));\n'
            '    println!("cargo:rerun-if-env-changed=CRYOLITE_NATIVE_CFLAGS");\n'
            '    let dst = cfg.build();')
    path = destination / "vendor/openzl/build/cmake/openzl-deps.cmake"
    text = path.read_text()
    start = text.index("# We aren't currently using pthreads")
    end = text.index("\nadd_library(openzl_deps INTERFACE)", start)
    block = text[start:end]
    path.write_text(text[:start] + "if(NOT OPENZL_NATIVE_NO_THREADS)\n" + block
                    + "\nendif()\n" + text[end:])
    stamp.write_text(recipe_stamp + "\n")
    native = tree_manifest(destination)
    for name in ("source-owner", "upstream.sha256"):
        native.pop(name, None)
    (destination / "native-manifest.json").write_text(json.dumps(native, indent=2) + "\n")
    return {"upstream_tree_sha256": fingerprint, "upstream_files": source_files,
            "native_build_rs_sha256": digest(destination / "build.rs"),
            "native_cmake_sha256": digest(path)}


def native_environment(build, provider, target, library, program):
    resource = subprocess.check_output(["clang", "-print-resource-dir"], text=True).strip()
    include = provider / "sysroot/usr/include"
    flags = ["--target=x86_64-unknown-none-elf", "-ffreestanding", "-fno-stack-protector",
             "-fno-pie", "-fno-pic", "-mno-red-zone", "-march=x86-64-v3", "-mno-mmx",
             "-O3", "-ffunction-sections", "-fdata-sections", "-fno-unwind-tables",
             "-nostdinc", "-isystem", resource + "/include", "-isystem", str(include)]
    toolchain = build / "native-toolchain.cmake"
    quoted = lambda value: json.dumps(str(value), ensure_ascii=False)
    toolchain.write_text("\n".join([
        'set(CMAKE_SYSTEM_NAME Generic)', 'set(CMAKE_SYSTEM_PROCESSOR x86_64)',
        'set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)',
        'set(CMAKE_C_COMPILER clang)', 'set(CMAKE_CXX_COMPILER clang++)',
        'set(CMAKE_ASM_COMPILER clang)', 'set(CMAKE_AR llvm-ar)',
        'set(CMAKE_RANLIB llvm-ranlib)',
        f'set(CMAKE_C_FLAGS_INIT {quoted(shlex.join(flags))})',
        f'set(CMAKE_CXX_FLAGS_INIT {quoted(shlex.join(flags) + " -nostdinc++")})',
        f'set(CMAKE_ASM_FLAGS_INIT {quoted(shlex.join(flags))})',
        f'set(CMAKE_FIND_ROOT_PATH {quoted(provider / "sysroot")})',
        'set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)',
        'set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)',
        'set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)',
        'set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)', '',
    ]))
    environment = os.environ.copy()
    environment.pop("RUSTFLAGS", None)
    environment.pop("CARGO_ENCODED_RUSTFLAGS", None)
    environment["__CARGO_TESTS_ONLY_SRC_ROOT"] = str(library)
    environment["CRYOLITE_STD_PROVIDER"] = str(provider)
    environment["CARGO_NET_OFFLINE"] = "true"
    environment["CRATE_CC_NO_DEFAULTS"] = "1"
    environment["CC_SHELL_ESCAPED_FLAGS"] = "1"
    environment["CC_" + target.stem.replace("-", "_")] = "clang"
    environment["AR_" + target.stem.replace("-", "_")] = "llvm-ar"
    environment["CFLAGS_" + target.stem.replace("-", "_")] = shlex.join(flags)
    environment["CRYOLITE_NATIVE_CFLAGS"] = shlex.join(flags)
    environment["CMAKE_TOOLCHAIN_FILE"] = str(toolchain)
    environment["BINDGEN_EXTRA_CLANG_ARGS"] = shlex.join(flags)
    wrapper = build / "native-rustc.py"
    wrapper.write_text('''#!/usr/bin/env python3
import os
import sys

compiler, *arguments = sys.argv[1:]
native = "--target" in arguments and arguments[arguments.index("--target") + 1] == '''
                       + repr(str(target)) + '''
links_std = any(argument == "--extern" and arguments[index + 1].split("=", 1)[0].split(":")[-1] == "std"
                for index, argument in enumerate(arguments[:-1]))
crate = arguments[arguments.index("--crate-name") + 1] if "--crate-name" in arguments else ""
std_crates = {"betafs_block", "betafs_core", "betafs_tree", "betafs_codec",
              "zstd", "zstd_safe", "rust_openzl", "rust_openzl_sys",
              "lz4_flex", "blake3", "constant_time_eq", "thiserror", ''' + repr(program.replace("-", "_")) + '''}
if native and links_std and crate in std_crates:
    arguments.append("-Zcrate-attr=feature(restricted_std)")
os.execv(compiler, [compiler, *arguments])
''')
    wrapper.chmod(0o755)
    environment["RUSTC_WRAPPER"] = str(wrapper)
    return environment
