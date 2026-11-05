#!/usr/bin/env python3
"""Verify native BetAFS barriers and reopen across two boots of one owned disk."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

from boot_smoke import FAULT, kernel_log, observed_checks, resident_memory
from check_betafs import check_output_links, source_snapshot

ROOT = Path(__file__).resolve().parents[1]
OWNER = "kryo-betafs-two-boot-v1\n"
MARKER = b"CRYOLITE-BLOCK-PROBE-V1"
RESERVED_BLOCK = MARKER.ljust(4096, b"\0")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def betafs_checks(log, phase):
    log = kernel_log(log)
    lines = log.splitlines()
    required = {
        "codecs": "BETAFS: native Raw Zstd LZ4 OpenZL and engine checks OK",
        "locks": "BETAFS: blocked lock owner death recovery OK",
        "retirement": "BETAFS: service driver and backing retirement OK",
        "files": ("BETAFS: file sync_data sync_all and directory barriers OK phase=write"
                  if phase == "format" else "BETAFS: reopened data and metadata OK phase=verify"),
    }
    return {name: lines.count(marker) == 1 for name, marker in required.items()}


def completion(log, checks):
    clean = kernel_log(log)
    fault = FAULT.search(clean)
    if fault:
        return "failed", f"guest reported {fault[1]}"
    if any(marker in clean for marker in ("BOOT: init terminated", "BOOT: supervisor terminated")):
        return "failed", "initial supervisor terminated"
    return ("passed", None) if all(checks.values()) else ("pending", None)


def validate_prebuilt(path, profile, betafs):
    metadata = json.loads(path.with_suffix(".build.json").read_text())
    if metadata["profile"] != profile or metadata["image_sha256"] != digest(path):
        raise ValueError("Prebuilt profile or image hash mismatch")
    if not metadata["betafs_unchanged"] or not metadata["openzl_algorithm_sources_unchanged"]:
        raise ValueError("Prebuilt changed independent algorithm sources")
    if metadata["betafs_source_files"] != betafs:
        raise ValueError("Prebuilt engine sources do not match the selected working tree")
    if not metadata.get("patched_registry_origins"):
        raise ValueError("Prebuilt lacks locked provenance for patched registry inputs")
    program = Path(metadata["program_manifest"]).parent
    for name, expected in metadata["program_sources"].items():
        if digest(program / name) != expected:
            raise ValueError(f"Prebuilt program changed: {name}")
    for name, expected in metadata["native_interface_sources"].items():
        if digest(ROOT / name) != expected:
            raise ValueError(f"Prebuilt native interface changed: {name}")
    for name, expected in metadata["tool_sources"].items():
        if digest(ROOT / "tools" / name) != expected:
            raise ValueError(f"Prebuilt recipe changed: {name}")
    return metadata


def run_command(command, output, environment):
    with output.open("w") as stream:
        subprocess.run(command, cwd=ROOT, env=environment, check=True,
                       stdout=stream, stderr=subprocess.STDOUT)


def boot(output, image, phase, arguments):
    directory = output / phase
    directory.mkdir()
    debug = directory / "debug.log"
    debug.write_text("")
    command = [
        "qemu-system-x86_64", "-M", "q35,memory-backend=ram", "-global", "hpet.msi=on",
        "-object", "memory-backend-ram,id=ram,size=512M,prealloc=off,reserve=off",
        "-overcommit", "mem-lock=off", "-accel", "tcg,thread=multi",
        "-smp", str(arguments.cpus), "-m", "512M", "-cpu", arguments.cpu,
        "-display", "none", "-serial", "none", "-monitor", "none", "-no-reboot",
        "-debugcon", f"file:{debug}", "-cdrom", str(image), "-boot", "d",
        "-device", "pcie-root-port,id=storage-port,bus=pcie.0,chassis=1,slot=5",
        "-drive", f"file={output / 'volume.raw'},format=raw,if=none,id=volume,cache=writeback",
        "-device", "virtio-blk-pci,drive=volume,disable-legacy=on,bus=storage-port",
    ]
    result = {"phase": phase, "command": command, "status": "failed", "checks": {},
              "guest_exit_verified": False,
              "scope": "QEMU TCG, actual VirtIO FLUSH and reopen; no physical power-loss proof"}
    started = time.monotonic()
    with (directory / "qemu.stdout.log").open("w") as stdout, (directory / "qemu.stderr.log").open("w") as stderr:
        process = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr)
        try:
            while True:
                log = debug.read_text(errors="replace")
                checks = observed_checks(log, arguments.cpu, arguments.normal, arguments.cpus)
                checks.update({"betafs_" + key: value for key, value in betafs_checks(log, phase).items()})
                result["checks"] = checks
                state, reason = completion(log, checks)
                if state != "pending":
                    result["status"] = state
                    if reason:
                        result["reason"] = reason
                    break
                if process.poll() is not None:
                    result["reason"] = f"QEMU exited early: {process.returncode}"
                    break
                if time.monotonic() - started > arguments.timeout:
                    result["reason"] = "guest timeout"
                    break
                usage = resident_memory(process.pid)
                if usage and max(usage["VmRSS"], usage["VmHWM"]) > 2 << 30:
                    result["reason"] = "QEMU exceeded two GiB resident memory"
                    break
                time.sleep(0.1)
        finally:
            if process.poll() is None:
                process.kill()
                result["stop"] = "host_sigkill_after_markers" if result["status"] == "passed" else "host_sigkill_on_failure"
            else:
                result["stop"] = "qemu_exited"
            process.wait(timeout=10)
            result["exit_code"] = process.returncode
    result["elapsed_seconds"] = round(time.monotonic() - started, 3)
    result["debug_sha256"] = digest(debug)
    (directory / "evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--betafs-source", type=Path, required=True)
    parser.add_argument("--source-cache", type=Path)
    parser.add_argument("--outputdir", type=Path, required=True)
    parser.add_argument("--profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--cpu", default="Haswell-noTSX")
    parser.add_argument("--cpus", type=int, default=1)
    parser.add_argument("--normal", action="store_true")
    parser.add_argument("--timeout", type=int, default=240)
    parser.add_argument("--prebuilt-service", type=Path)
    parser.add_argument("--prebuilt-codecs", type=Path)
    arguments = parser.parse_args()
    if not 1 <= arguments.cpus <= 255 or arguments.timeout <= 0:
        parser.error("CPU count or timeout is invalid")
    if bool(arguments.prebuilt_service) != bool(arguments.prebuilt_codecs):
        parser.error("Supply both prebuilt programs or neither")
    source = arguments.betafs_source.resolve(strict=True)
    check_output_links(arguments.outputdir)
    output = arguments.outputdir.resolve()
    if output.is_relative_to(source) or source.is_relative_to(output):
        parser.error("Output must be separate from the independent engine")
    if output.exists() and (not (output / "owner").is_file() or (output / "owner").read_text() != OWNER):
        parser.error("Output directory is not owned by this verifier")
    if (output / "volume.raw").exists():
        parser.error("Existing disk/logs are preserved; select a fresh output directory")
    output.mkdir(parents=True, exist_ok=True)
    (output / "owner").write_text(OWNER)
    before = source_snapshot(source)
    result = {"status": "failed", "scope": "Native Ring3 BetAFS over real block IPC, two boots of one disposable volume",
              "engine_sources_before": before, "profile": arguments.profile, "cpus": arguments.cpus,
              "cpu": arguments.cpu, "normal": arguments.normal, "boots": []}
    environment = os.environ.copy()
    if arguments.source_cache:
        environment["CRYOLITE_LIBC_CACHE"] = str(arguments.source_cache.resolve())
    make = ["make", f"BUILD_DIR={output}", f"CARGO_TARGET_DIR={output / 'kernel-target'}",
            f"PROFILE={arguments.profile}", "FEATURES=" + ("" if arguments.normal else "boot-tests"),
            "BETAFS_PROBE=1", "STD_PROBE=0", "BLOCK_PROBE=0", f"BETAFS_SOURCE={source}"]
    try:
        if arguments.prebuilt_service:
            result["prebuilt"] = {}
            for name, path in [("betafs-service", arguments.prebuilt_service), ("betafs-codecs", arguments.prebuilt_codecs)]:
                metadata = validate_prebuilt(path.resolve(strict=True), arguments.profile, before)
                shutil.copy2(path, output / (name + ".elf"))
                shutil.copy2(path.with_suffix(".build.json"), output / (name + ".build.json"))
                result["prebuilt"][name] = metadata["image_sha256"]
            make.append("BETAFS_PREBUILT=1")
        print("BetAFS: building native boot image", flush=True)
        run_command([*make, "iso", "BETAFS_PHASE=format"], output / "build.log", environment)
        disk = output / "volume.raw"
        with disk.open("xb") as stream:
            stream.write(RESERVED_BLOCK)
            stream.truncate(32 << 20)
        initial = digest(disk)
        for phase in ("format", "open"):
            if phase == "open":
                run_command([*make, "image", "BETAFS_PHASE=open"], output / "repack.log", environment)
            print(f"BetAFS: booting {phase} phase", flush=True)
            run = boot(output, output / "kryo.iso", phase, arguments)
            run["disk_sha256"] = digest(disk)
            result["boots"].append(run)
            with disk.open("rb") as stream:
                if stream.read(4096) != RESERVED_BLOCK:
                    raise RuntimeError("Reserved marker block was modified")
            if run["status"] != "passed":
                raise RuntimeError(f"{phase} boot failed: {run.get('reason')}")
        if initial == result["boots"][0]["disk_sha256"]:
            raise RuntimeError("Filesystem write phase did not change the disk")
        result["status"] = "passed"
        result["marker_preserved"] = True
        result["disk_recreated_between_boots"] = False
    except (OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as error:
        result["error"] = str(error)
    finally:
        result["engine_sources_after"] = source_snapshot(source)
        result["engine_unchanged"] = before == result["engine_sources_after"]
        if not result["engine_unchanged"]:
            result["status"] = "failed"
            result["error"] = "Independent engine sources changed"
        (output / "evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    print(f"BetAFS: {result['status']}; evidence: {output / 'evidence.json'}", flush=True)
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
