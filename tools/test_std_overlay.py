import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import std_overlay


class SourceOwnershipTests(unittest.TestCase):
    def setUp(self):
        temporary = std_overlay.ROOT / "build"
        temporary.mkdir(exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(dir=temporary)
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        self.source = root / "source"
        self.libc = root / "libc"
        self.port = root / "port"
        self.build = root / "output"
        self.source.mkdir()
        (self.libc / "src/new").mkdir(parents=True)
        (self.port / "overlay").mkdir(parents=True)
        (self.source / "lib.rs").write_text("pinned Rust\n")
        (self.libc / "src/lib.rs").write_text("pinned libc\n")
        (self.libc / "src/new/mod.rs").write_text("pinned libc module\n")
        pins = {
            "library_sha256": std_overlay.tree_digest(self.source),
            "libc_sha256": std_overlay.tree_digest(self.libc),
            "source_files": {"lib.rs": std_overlay.digest(self.source / "lib.rs")},
            "libc_root_sha256": std_overlay.digest(self.libc / "src/lib.rs"),
            "libc_new_sha256": std_overlay.digest(self.libc / "src/new/mod.rs"),
        }
        (self.port / "sources.json").write_text(json.dumps(pins))
        self.enterContext(patch.object(std_overlay, "PORT", self.port))
        self.enterContext(patch.object(std_overlay, "apply", self.apply))

    @staticmethod
    def apply(library, libc):
        (library / "port.rs").write_text("owned overlay\n")

    def prepare(self):
        return std_overlay.prepare(self.build, self.source, self.libc)

    def test_changed_generated_rust_is_rebuilt(self):
        library = self.prepare()
        (library / "lib.rs").write_text("unexpected code\n")
        self.prepare()
        self.assertEqual((library / "lib.rs").read_text(), "pinned Rust\n")

    def test_changed_generated_libc_is_rebuilt(self):
        self.prepare()
        path = self.build / "rust/libc/src/lib.rs"
        path.write_text("unexpected bindings\n")
        self.prepare()
        self.assertEqual(path.read_text(), "pinned libc\n")

    def test_unowned_output_is_preserved(self):
        library = self.build / "rust/library"
        library.mkdir(parents=True)
        (library / "human.rs").write_text("keep\n")
        with self.assertRaisesRegex(RuntimeError, "unowned"):
            self.prepare()
        self.assertEqual((library / "human.rs").read_text(), "keep\n")

    def test_changed_input_is_rejected_before_output(self):
        (self.source / "lib.rs").write_text("different compiler source\n")
        with self.assertRaisesRegex(RuntimeError, "does not match"):
            self.prepare()
        self.assertFalse(self.build.exists())

    def test_source_symlink_is_rejected(self):
        (self.source / "escape.rs").symlink_to(self.libc / "src/lib.rs")
        with self.assertRaisesRegex(RuntimeError, "symlink"):
            self.prepare()


if __name__ == "__main__":
    unittest.main()
