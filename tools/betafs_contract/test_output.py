"""Exercise verifier write containment before any Cargo work can begin."""

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


VERIFIER = Path(__file__).resolve().parents[1] / "check_betafs.py"


class OutputContainmentTests(unittest.TestCase):
    def run_rejection(self, kind):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            betafs = root / "betafs"
            marker = betafs / "crates/betafs-core/Cargo.toml"
            marker.parent.mkdir(parents=True)
            marker.write_text('[package]\nname="betafs-core"\nversion="0.0.0"\n')
            (betafs / "Cargo.lock").write_text("version = 4\n")
            subprocess.run(["rtk", "proxy", "git", "init", "--quiet", str(betafs)], check=True)
            output = root / "output"
            output.mkdir()
            owner = output / "owner"
            owner.write_text("kryo-betafs-contract-v1\n")
            victim = root / "outside/src/lib.rs"
            victim.parent.mkdir(parents=True)
            victim.write_text("protected source\n")
            if kind == "directory":
                (output / "fixture").symlink_to(victim.parent.parent, target_is_directory=True)
            elif kind == "owner":
                owner.unlink()
                victim.write_text("kryo-betafs-contract-v1\n")
                owner.symlink_to(victim)
            else:
                target = output / "fixture/src/lib.rs"
                target.parent.mkdir(parents=True)
                if kind == "dangling":
                    victim.unlink()
                target.symlink_to(victim)
            previous = victim.read_bytes() if victim.exists() else None
            owner_time = owner.stat().st_mtime_ns
            command = ["rtk", "proxy", sys.executable, str(VERIFIER),
                       "--betafs-root", str(betafs), "--outputdir", str(output)]
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("symlink", result.stderr)
            self.assertEqual(victim.read_bytes() if victim.exists() else None, previous)
            self.assertEqual(owner.stat().st_mtime_ns, owner_time)
            self.assertFalse((output / "fixture/Cargo.toml").exists())

    def test_external_file_is_never_overwritten(self):
        self.run_rejection("file")

    def test_external_directory_is_never_written(self):
        self.run_rejection("directory")

    def test_dangling_external_link_is_rejected(self):
        self.run_rejection("dangling")

    def test_owner_link_is_rejected_before_write(self):
        self.run_rejection("owner")

    def test_contained_links_are_allowed(self):
        spec = importlib.util.spec_from_file_location("check_betafs", VERIFIER)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "actual").write_text("cache\n")
            (root / "link").symlink_to(root / "actual")
            module.check_output_links(root)


if __name__ == "__main__":
    unittest.main()
