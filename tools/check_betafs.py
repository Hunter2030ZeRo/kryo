#!/usr/bin/env python3
"""Check the independent BetAFS engine against host models of Kryo contracts."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[1]
HARNESS = ROOT / "tools/betafs_contract/contract.rs"
HOST = "x86_64-unknown-linux-gnu"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git(root, *arguments):
    return subprocess.check_output(["rtk", "proxy", "git", "-C", str(root), *arguments])


def source_snapshot(root):
    names = set(git(root, "ls-files", "-z").decode().split("\0"))
    names.update(git(root, "ls-files", "--others", "--exclude-standard", "-z").decode().split("\0"))
    return {name: digest(root / name) for name in sorted(names) if name and (root / name).is_file()}


def check_output_links(output):
    if (output / "owner").is_symlink():
        raise ValueError("Output owner must not be a symlink")
    for directory, directories, files in os.walk(output, followlinks=False):
        for name in directories + files:
            path = Path(directory) / name
            if path.is_symlink():
                try:
                    target = path.resolve()
                except (OSError, RuntimeError) as error:
                    raise ValueError(f"Cannot resolve output symlink: {path}") from error
                if not target.is_relative_to(output):
                    raise ValueError(f"Output symlink escapes owned directory: {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--betafs-root", type=Path, required=True)
    parser.add_argument("--outputdir", type=Path, required=True)
    arguments = parser.parse_args()
    betafs = arguments.betafs_root.resolve(strict=True)
    output = arguments.outputdir.resolve()
    if output.is_relative_to(betafs) or betafs.is_relative_to(output):
        parser.error("Output must be separate from the independent BetAFS repository")
    if not (betafs / "crates/betafs-core/Cargo.toml").is_file():
        parser.error("Expected the independent BetAFS workspace")
    owner = output / "owner"
    try:
        check_output_links(output)
    except ValueError as error:
        parser.error(str(error))
    if output.exists() and (not owner.is_file() or owner.read_text() != "kryo-betafs-contract-v1\n"):
        parser.error("Refusing an output directory not owned by this verifier")
    output.mkdir(parents=True, exist_ok=True)
    owner.write_text("kryo-betafs-contract-v1\n")
    before = source_snapshot(betafs)
    status = git(betafs, "status", "--porcelain=v1").decode()
    project = output / "fixture"
    (project / "src").mkdir(parents=True, exist_ok=True)
    (project / "src/lib.rs").write_bytes(HARNESS.read_bytes())
    quoted = lambda path: json.dumps(str(path), ensure_ascii=False)
    manifest = [
        '[package]', 'name = "kryo-betafs-contract"', 'version = "0.0.0"',
        'edition = "2024"', 'publish = false', '', '[workspace]', 'resolver = "3"', '', '[dependencies]',
        *[f'{name} = {{ path = {quoted(betafs / "crates" / name)} }}'
          for name in ("betafs-block", "betafs-core", "betafs-tree")],
        f'kryo-interfaces = {{ path = {quoted(ROOT / "interfaces")} }}', '',
    ]
    (project / "Cargo.toml").write_text("\n".join(manifest))
    # Cargo may update only this isolated lock to add the verifier package.
    (project / "Cargo.lock").write_bytes((betafs / "Cargo.lock").read_bytes())
    environment = os.environ.copy()
    environment.pop("RUSTFLAGS", None)
    environment.pop("CARGO_ENCODED_RUSTFLAGS", None)
    environment["CARGO_TARGET_DIR"] = str(output / "target")
    environment["CARGO_NET_OFFLINE"] = "true"
    commands = []
    result = {
        "scope": "Actual host BetAFS engine over a fault-injected Kryo wire model; not native IPC, guest BetAFS, DMA or physical durability evidence",
        "betafs_root": str(betafs), "betafs_head": git(betafs, "rev-parse", "HEAD").decode().strip(),
        "betafs_dirty_status": status, "betafs_source_before": before,
        "seed_lock_sha256": digest(betafs / "Cargo.lock"),
        "harness_sha256": digest(HARNESS), "verifier_sha256": digest(Path(__file__)),
        "interface_sources": {str(p.relative_to(ROOT)): digest(p)
                              for p in sorted((ROOT / "interfaces").rglob("*"))
                              if p.is_file() and p.suffix in (".rs", ".toml")},
        "commands": commands, "status": "failed",
    }

    def run(name, arguments, capture=False):
        log = output / (name + ".log")
        with log.open("w") as stream:
            completed = subprocess.run(["rtk", "proxy", *map(str, arguments)], cwd=project,
                                       env=environment, stdout=stream, stderr=subprocess.STDOUT)
        commands.append({"name": name, "arguments": list(map(str, arguments)),
                         "exit_code": completed.returncode, "log": log.name,
                         "log_sha256": digest(log)})
        if completed.returncode:
            print("\n".join(log.read_text().splitlines()[-35:]), file=sys.stderr)
            raise RuntimeError(f"{name} failed; see {log}")
        return log.read_text() if capture else None

    try:
        result["rustc"] = run("rustc", ["rustc", "--version", "--verbose"], True).strip()
        result["cargo"] = run("cargo", ["cargo", "--version"], True).strip()
        run("resolve", ["cargo", "metadata", "--offline", "--format-version", "1", "--manifest-path", project / "Cargo.toml"])
        result["resolved_lock_sha256"] = digest(project / "Cargo.lock")
        seed = tomllib.loads((betafs / "Cargo.lock").read_text())["package"]
        resolved = tomllib.loads((project / "Cargo.lock").read_text())["package"]
        key = lambda package: (package["name"], package["version"], package.get("source"), package.get("checksum"))
        upstream = {key(package) for package in seed if "source" in package}
        result["resolved_upstream_packages"] = [key(package) for package in resolved if "source" in package]
        if any(tuple(package) not in upstream for package in result["resolved_upstream_packages"]):
            raise RuntimeError("Resolver drifted from the BetAFS dependency lock")
        text = run("tests", ["cargo", "test", "--manifest-path", project / "Cargo.toml", "--target", HOST,
                              "--locked", "--offline", "--", "--test-threads=1"], True)
        result["tests"] = re.findall(r"(?m)^test ([\w:]+) \.\.\. ok$", text)
        if len(result["tests"]) != 15:
            raise RuntimeError("Expected all fifteen contract tests")
        result["status"] = "passed"
    except (RuntimeError, OSError) as error:
        result["error"] = str(error)
    finally:
        after = source_snapshot(betafs)
        result["betafs_source_after"] = after
        result["betafs_unchanged"] = before == after and status == git(betafs, "status", "--porcelain=v1").decode()
        result["verification_inputs_unchanged"] = (
            result["harness_sha256"] == digest(HARNESS)
            and result["verifier_sha256"] == digest(Path(__file__))
            and all(digest(ROOT / name) == value for name, value in result["interface_sources"].items())
        )
        if not result["verification_inputs_unchanged"]:
            result["status"] = "failed"
            result["error"] = "Verifier or interface inputs changed during execution"
        if not result["betafs_unchanged"]:
            result["status"] = "failed"
            result["error"] = "Independent BetAFS source changed during verification"
        (output / "evidence.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    print(f"BetAFS contract: {result['status']}; evidence: {output / 'evidence.json'}")
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
