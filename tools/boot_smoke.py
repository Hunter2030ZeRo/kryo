#!/usr/bin/env python3
"""Build and verify one headless QEMU boot without replacing normal boot logs."""

import argparse
import json
import re
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BOOT_TIMEOUT = 30
MARKERS = {
    "frames": "FRAME: alloc/release OK",
    "zeroing": "RAM: zero OK",
    "paging": "PAGING: map/unmap OK cycles=128",
    "user_spaces": "USER: separate address spaces OK",
    "user_isolation": "USER: isolation OK cases=16",
    "vm_rollback": "VM: alias preparation rollback OK",
    "preemption": "TIMER: preemption OK switches=8",
    "capabilities": "CAP: checks OK denied=3 stale=1",
    "timer_drain": "TIMER: pending interrupt drained",
    "task_teardown": "TASK: teardown OK",
    "invalid_context": "TASK: invalid saved state isolated",
    "ipc": "IPC: rendezvous and lifetime OK cases=8",
    "idle": "IDLE: halt and wake OK cases=3",
}
FAULT = re.compile(r"(?m)^(PANIC|#PF|#GP)(?=\s|$)")


def observed_checks(log):
    checks = {
        name: bool(re.search(r"(?m)^" + re.escape(marker) + r"(?=\s|$)", log))
        for name, marker in MARKERS.items()
    }
    checks["breakpoint"] = bool(re.search(r"(?:^|\n)ABC\s*\Z", log))
    return checks


def stop_qemu(process):
    if process.poll() is not None:
        return "already_exited"
    try:
        process.terminate()
    except ProcessLookupError:
        return "already_exited"
    try:
        process.wait(timeout=3)
        return "host_sigterm"
    except subprocess.TimeoutExpired:
        try:
            process.kill()
        except ProcessLookupError:
            return "already_exited"
        try:
            process.wait(timeout=3)
            return "host_sigkill"
        except subprocess.TimeoutExpired:
            return "host_sigkill_timeout"


def run_profile(profile):
    output = ROOT / "build" / "smoke" / profile
    output.mkdir(parents=True, exist_ok=True)
    logs = {
        name: output / filename
        for name, filename in {
            "build_stdout": "build.stdout.log",
            "build_stderr": "build.stderr.log",
            "debug": "debug.log",
            "stdout": "qemu.stdout.log",
            "stderr": "qemu.stderr.log",
        }.items()
    }
    evidence = {
        "profile": profile,
        "status": "failed",
        "reason": "build not completed",
        "checks": observed_checks(""),
        "logs": {name: str(path) for name, path in logs.items()},
        "guest_exit_verified": False,
        "scope": "QEMU TCG boot markers; host termination does not prove guest exit",
    }
    process = None
    boot_started = None
    started = time.monotonic()

    try:
        print(f"{profile}: building ISO", flush=True)
        with logs["build_stdout"].open("w") as stdout, logs["build_stderr"].open("w") as stderr:
            build = subprocess.run(
                ["make", "iso", f"BUILD_DIR={output}", f"PROFILE={profile}", "FEATURES=boot-tests"],
                cwd=ROOT, stdout=stdout, stderr=stderr, check=False,
            )
        evidence["build_exit_code"] = build.returncode
        if build.returncode:
            evidence["reason"] = f"ISO build exited with {build.returncode}"
            return evidence

        logs["debug"].write_text("")
        command = [
            "qemu-system-x86_64", "-M", "q35", "-accel", "tcg", "-m", "512M",
            "-display", "none", "-serial", "none", "-monitor", "none", "-no-reboot",
            "-debugcon", f"file:{logs['debug']}",
            "-cdrom", str(output / "kryo.iso"), "-boot", "d",
        ]
        evidence["qemu_command"] = command
        print(f"{profile}: booting QEMU ({BOOT_TIMEOUT}s limit)", flush=True)
        with logs["stdout"].open("w") as stdout, logs["stderr"].open("w") as stderr:
            process = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr)
            evidence["qemu_pid"] = process.pid
            boot_started = time.monotonic()
            while True:
                log = logs["debug"].read_text(errors="replace")
                evidence["checks"] = observed_checks(log)
                fault = FAULT.search(log)
                if fault:
                    evidence["reason"] = f"guest reported {fault[1]}"
                    break
                if all(evidence["checks"].values()):
                    evidence["status"] = "passed"
                    evidence["reason"] = "all boot markers observed"
                    break
                if process.poll() is not None:
                    evidence["reason"] = f"QEMU exited before all markers ({process.returncode})"
                    break
                if time.monotonic() - boot_started >= BOOT_TIMEOUT:
                    evidence["reason"] = f"boot timed out after {BOOT_TIMEOUT}s"
                    break
                time.sleep(0.05)
    except KeyboardInterrupt:
        evidence["status"] = "interrupted"
        evidence["reason"] = "interrupted by user"
    except OSError as error:
        evidence["reason"] = str(error)
    finally:
        if boot_started is not None:
            evidence["boot_seconds"] = round(time.monotonic() - boot_started, 3)
        if process is not None:
            evidence["qemu_stop"] = stop_qemu(process)
            evidence["qemu_exit_code"] = process.poll()
            if evidence["qemu_stop"] == "host_sigkill_timeout":
                evidence["status"] = "failed"
                evidence["reason"] = "QEMU did not stop after SIGKILL"
        evidence["elapsed_seconds"] = round(time.monotonic() - started, 3)
        (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")

    return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--profile", choices=("debug", "release"), default="debug")
    selection.add_argument("--all", action="store_true", help="test debug and release once each")
    arguments = parser.parse_args()
    profiles = ("debug", "release") if arguments.all else (arguments.profile,)
    passed = True
    for profile in profiles:
        result = run_profile(profile)
        print(f"{profile}: {result['status']} — {result['reason']}")
        print(f"  evidence: {ROOT / 'build' / 'smoke' / profile / 'evidence.json'}")
        if result["status"] == "interrupted":
            return 130
        passed &= result["status"] == "passed"
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
