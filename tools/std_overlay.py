"""Apply the Cryolite port to an owned copy of the pinned Rust sources."""

import hashlib
import json
from pathlib import Path
import shutil


ROOT = Path(__file__).resolve().parents[1]
PORT = ROOT / "userspace/rust-std"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tree_manifest(root):
    result = {}
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise RuntimeError(f"Source tree contains a symlink: {path}")
        if path.is_file():
            result[path.relative_to(root).as_posix()] = digest(path)
    return result


def tree_digest(root):
    return hashlib.sha256(json.dumps(tree_manifest(root), sort_keys=True).encode()).hexdigest()


def replace(path, old, new, count=1):
    text = path.read_text()
    if text.count(old) != count:
        raise RuntimeError(f"Pinned source context changed: {path}: {old!r}")
    path.write_text(text.replace(old, new))


def branch(library, name, body, *, occurrence=1):
    path = library / "std/src" / name
    text = path.read_text()
    start = -1
    for _ in range(occurrence):
        start = text.index("cfg_select! {", start + 1)
    end = start + len("cfg_select! {")
    path.write_text(text[:end] + '\n    target_os = "cryolite" => {\n' + body
                    + "\n    }" + text[end:])


def apply(library, libc):
    overlay = PORT / "overlay"
    shutil.copytree(overlay / "library", library, dirs_exist_ok=True)
    shutil.copytree(overlay / "libc", libc, dirs_exist_ok=True)
    for name in ("pal", "thread", "args", "stdio", "random", "io/error", "sync/futex"):
        branch(library, f"sys/{name}/mod.rs", "        mod cryolite;\n        pub use cryolite::*;")
    branch(library, "sys/time/mod.rs", "        mod cryolite;\n        use cryolite as imp;")
    branch(library, "sys/fd/mod.rs", "")
    branch(library, "sys/paths/mod.rs", "        mod unsupported; use unsupported as imp;")
    branch(library, "sys/personality/mod.rs", """        #[lang = "eh_personality"]
        fn rust_eh_personality() {
            core::intrinsics::abort_immediate()
        }""")
    branch(library, "sys/alloc/mod.rs", "        mod cryolite;\n        use cryolite as imp;")
    for name in ("mutex", "condvar", "once", "rwlock", "thread_parking"):
        path = library / f"std/src/sys/sync/{name}/mod.rs"
        text = path.read_text()
        text = text.replace('    any(\n', '    any(\n        target_os = "cryolite",\n', 1)
        path.write_text(text)
    branch(library, "sys/thread_local/mod.rs", """        mod racy;
        mod cryolite;
        pub(super) use racy::LazyKey;
        pub(super) use cryolite::{Key, set};
        #[cfg(any(not(target_thread_local), test))]
        pub(super) use cryolite::get;
        use cryolite::{create, destroy};""", occurrence=4)
    for name, body in {
        "fs": "mod unsupported; use unsupported as imp;",
        "env": "mod unsupported; pub use unsupported::*;",
        "process": "mod unsupported; use unsupported as imp;",
        "pipe": "mod unsupported; pub use unsupported::{Pipe, pipe};",
        "net/connection": "mod unsupported; pub use unsupported::*;",
        "net/hostname": "mod unsupported; pub use unsupported::hostname;",
    }.items():
        branch(library, f"sys/{name}/mod.rs", "        " + body)
    branch(library, "sys/io/mod.rs", "        mod unsupported;\n        pub use unsupported::*;")
    path = library / "std/src/sys/fs/mod.rs"
    replace(path, '#[cfg(not(any(target_family = "unix", target_os = "windows", target_os = "wasi")))]',
            '#[cfg(any(target_os = "cryolite", not(any(target_family = "unix", target_os = "windows", target_os = "wasi"))))]')
    path = library / "std/src/sys/process/mod.rs"
    replace(path, '#[cfg(target_family = "unix")]', '#[cfg(all(target_family = "unix", not(target_os = "cryolite")))]')
    replace(path, '            target_os = "l4re"', '            target_os = "l4re",\n            target_os = "cryolite"', count=2)
    branch(library, "os/mod.rs", '        #[path = "cryolite/unix.rs"]\n        pub mod unix;', occurrence=2)
    replace(library / "std/src/os/mod.rs", 'pub mod fd;', '#[cfg_attr(target_os = "cryolite", path = "cryolite/fd.rs")]\npub mod fd;')
    replace(library / "unwind/src/lib.rs", 'any(target_os = "none", target_os = "espidf", target_os = "nuttx")',
            'any(target_os = "cryolite", target_os = "none", target_os = "espidf", target_os = "nuttx")')
    path = library / "panic_unwind/src/lib.rs"
    text = path.read_text().replace('cfg_select! {', '''cfg_select! {
    target_os = "cryolite" => {
        #[path = "dummy.rs"]
        mod imp;
    }''', 1)
    path.write_text(text)
    replace(library / "backtrace/src/backtrace/mod.rs", '                not(target_os = "emscripten"),',
            '                not(target_os = "emscripten"),\n                not(target_os = "cryolite"),')
    replace(libc / "src/lib.rs", '    } else if #[cfg(unix)] {', '''    } else if #[cfg(target_os = "cryolite")] {
        mod primitives;
        pub use crate::primitives::*;
        mod cryolite;
        pub use crate::cryolite::*;
        prelude!();
    } else if #[cfg(unix)] {''')
    replace(libc / "src/new/mod.rs", 'all(target_family = "unix", not(target_os = "qurt"))',
            'all(target_family = "unix", not(any(target_os = "qurt", target_os = "cryolite")))')
    with (library / "Cargo.toml").open("a") as file:
        file.write(f'libc = {{ path = {json.dumps(str(libc))} }}\n')


def prepare(build, source, libc_source):
    pins = json.loads((PORT / "sources.json").read_text())
    if tree_digest(source) != pins["library_sha256"]:
        raise RuntimeError("Rust library source tree does not match the pinned port")
    if tree_digest(libc_source) != pins["libc_sha256"]:
        raise RuntimeError("libc source tree does not match the pinned port")
    for name, expected in pins["source_files"].items():
        if digest(source / name) != expected:
            raise RuntimeError(f"Rust source does not match the pinned port: {name}")
    if digest(libc_source / "src/lib.rs") != pins["libc_root_sha256"]:
        raise RuntimeError("libc source does not match the pinned port")
    if digest(libc_source / "src/new/mod.rs") != pins["libc_new_sha256"]:
        raise RuntimeError("libc new module does not match the pinned port")
    library = build / "rust/library"
    libc = build / "rust/libc"
    owner = build / "rust/source-owner"
    stamp = build / "rust/overlay-inputs.sha256"
    manifest = build / "rust/source-manifest.json"
    inputs = [Path(__file__), PORT / "sources.json", *sorted((PORT / "overlay").rglob("*"))]
    fingerprint = hashlib.sha256(json.dumps([
        (str(p.relative_to(ROOT)), digest(p)) for p in inputs if p.is_file()
    ]).encode()).hexdigest()
    if library.exists():
        if not owner.is_file() or owner.read_text() != "cryolite-rust-std-v1\n":
            raise RuntimeError(f"Refusing to replace unowned Rust sources: {library}")
        if (stamp.is_file() and stamp.read_text().strip() == fingerprint
                and manifest.is_file() and json.loads(manifest.read_text()) == {
                    "library": tree_manifest(library), "libc": tree_manifest(libc)
                }):
            return library
        shutil.rmtree(library)
        shutil.rmtree(libc)
    shutil.copytree(source, library, symlinks=True)
    shutil.copytree(libc_source, libc, symlinks=True)
    owner.write_text("cryolite-rust-std-v1\n")
    apply(library, libc)
    manifest.write_text(json.dumps({"library": tree_manifest(library),
                                    "libc": tree_manifest(libc)}, indent=2) + "\n")
    stamp.write_text(fingerprint + "\n")
    return library
