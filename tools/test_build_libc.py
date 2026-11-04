import hashlib
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import build_libc


class LibcSourceTests(unittest.TestCase):
    def test_fingerprint_includes_relative_file_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first, renamed = root / "first.h", root / "renamed.h"
            first.write_text("same contents")
            with patch.object(build_libc, "ROOT", root):
                before = build_libc.input_fingerprint([first])
                first.rename(renamed)
                self.assertNotEqual(before, build_libc.input_fingerprint([renamed]))

    def test_cached_archive_hash_is_checked_without_network(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory)
            archive = cache / "test-revision.tar.gz"
            archive.write_bytes(b"original source")
            pin = {"revision": "revision", "repository": "https://github.com/example/test",
                   "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest()}
            self.assertEqual(build_libc.archive(cache, "test", pin, False), archive)
            archive.write_bytes(b"changed source")
            with self.assertRaisesRegex(RuntimeError, "Cached source hash mismatch"):
                build_libc.archive(cache, "test", pin, False)

    def test_manifest_detects_edits_removals_and_link_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            header = root / "header.h"
            header.write_text("source")
            link = root / "alias.h"
            link.symlink_to("header.h")
            before = build_libc.source_manifest(root)
            header.write_text("modified")
            self.assertNotEqual(before, build_libc.source_manifest(root))
            header.write_text("source")
            link.unlink()
            link.symlink_to("other.h")
            self.assertNotEqual(before, build_libc.source_manifest(root))
            link.unlink()
            self.assertNotEqual(before, build_libc.source_manifest(root))

    def test_unowned_source_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            build = Path(directory)
            source = build / "source"
            source.mkdir()
            keep = source / "user-file"
            keep.write_text("keep")
            with self.assertRaisesRegex(RuntimeError, "unowned source tree"):
                build_libc.prepare(build, build / "cache", False)
            self.assertEqual(keep.read_text(), "keep")

    def test_archive_cannot_write_outside_its_extraction_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "bad.tar.gz"
            with tarfile.open(archive, "w:gz") as target:
                member = tarfile.TarInfo("../../outside")
                member.size = 1
                target.addfile(member, io.BytesIO(b"x"))
            with self.assertRaises((RuntimeError, tarfile.FilterError)):
                build_libc.extract(archive, root / "generated/source")
            self.assertFalse((root / "outside").exists())


if __name__ == "__main__":
    unittest.main()
