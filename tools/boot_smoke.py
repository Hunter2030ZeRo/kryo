#!/usr/bin/env python3
"""Build and verify one headless QEMU boot without replacing normal boot logs."""

import argparse
import json
import re
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BOOT_TIMEOUT = 90
MARKERS = {
    "heap": "HEAP: alignment/reuse/failure OK",
    "growth": "SCALE: native growth OK processes=7 threads=127 waits=127",
    "growth_oom": "SCALE: process publication OOM retained owners",
    "growth_retirement": "SCALE: native waits and process resources retired",
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
    "shared_memory": "SHARED: mapping and lifetime OK pages=3",
    "cap_transfer": "TRANSFER: commit and rollback OK cases=2",
    "shared_fault": "SHARED: read-only fault isolated vector=14 error=7",
    "elf_native": "ELF: native execution OK tasks=2",
    "extended_state": "STATE: x87 SIMD TLS isolation OK",
    "clock": "CLOCK: HPET monotonic ready",
    "deadlines": "TIME: deadlines and wakeup OK cases=11",
    "early_irq": "TIME: early IRQ ignored until deadline",
    "launch": "LAUNCH: rejection and rollback OK cases=9",
    "spawn": "SPAWN: publication rollback OK cases=5",
    "threads": "THREAD: lifecycle and isolation OK cases=9",
    "thread_state": "THREAD: shared-process state OK threads=4 preempted=4",
    "supervision": "INIT: user supervision and restart OK generations=2",
    "device_queue": "DEVICE: queued IRQ retired before reuse",
    "device_expiry": "DEVICE: expired wait precedes queued IRQ",
    "device_retry": "DEVICE: retirement retry retains owners",
    "channel_creation": "CHANNEL: creation rollback and lifetime OK",
    "channel_retirement": "IPC: channel registry retirement and retained charges OK",
    "native_services": "SERVICE: native IPC and restart OK instances=2 quota=8",
    "startup_authority": "STARTUP: authority and private grant rollback OK",
    "startup_publication": "STARTUP: spawn publication rollback OK cases=3",
    "supervisor_authority": "SUPERVISOR: manifests and authority isolation OK instances=2",
    "relative_receive": "RECEIVE: relative admission and absolute expiry OK",
}
FAULT = re.compile(r"(?m)^(PANIC|#PF|#GP)(?=\s|$)")


def parse_memory(value):
    match = re.fullmatch(r"([1-9][0-9]*)([MG])", value.upper())
    if not match:
        raise argparse.ArgumentTypeError("RAM must be a positive size such as 512M or 128G")
    size, unit = match.groups()
    if int(size) * (1 << (20 if unit == "M" else 30)) >= 1 << 63:
        raise argparse.ArgumentTypeError("RAM size exceeds the supported argument range")
    return f"{int(size)}{unit}"


def memory_bytes(value):
    value = parse_memory(value)
    return int(value[:-1]) * (1 << (20 if value[-1] == "M" else 30))


def memory_details(log):
    details = {"metadata": None, "managed_frames": None, "available": [], "high_frames": [], "tlb_backing": None}
    metadata = re.search(
        r"(?m)^FRAME metadata: start=(0x[0-9a-f]+) end=(0x[0-9a-f]+) bytes=(\d+) "
        r"bitmap_words=(\d+) summary_words=(\d+) usable_frames=(\d+) managed_frames=(\d+)$", log)
    if metadata:
        keys = ("start", "end", "bytes", "bitmap_words", "summary_words", "usable_frames", "managed_frames")
        values = [int(value, 16 if index < 2 else 10) for index, value in enumerate(metadata.groups())]
        details["metadata"] = dict(zip(keys, values))
    ready = re.search(r"(?m)^FRAME: ready total=(\d+) free=(\d+)$", log)
    if ready:
        details["managed_frames"] = int(ready[1])
    for start, end in re.findall(r"(?m)^AVAILABLE\[\d+\]: \[(0x[0-9a-f]+), (0x[0-9a-f]+)\)", log):
        details["available"].append([int(start, 16), int(end, 16)])
    for minimum, physical in re.findall(r"(?m)^RAM: high frame OK minimum=(0x[0-9a-f]+) physical=(0x[0-9a-f]+)$", log):
        details["high_frames"].append({"minimum": int(minimum, 16), "physical": int(physical, 16)})
    tlb = re.search(r"(?m)^SMP: TLB backing old=(0x[0-9a-f]+) new=(0x[0-9a-f]+)$", log)
    if tlb:
        details["tlb_backing"] = [int(value, 16) for value in tlb.groups()]
    return details


def memory_checks(log, normal):
    details = memory_details(log)
    value = details["metadata"]
    valid = False
    if value:
        span = value["end"] - value["start"]
        areas = details["available"]
        valid = (
            span > 0 and value["start"] % 4096 == 0 and span % 4096 == 0
            and span == value["bytes"]
            and (value["bitmap_words"] + value["summary_words"]) * 8 <= span
            and value["bitmap_words"] == (value["usable_frames"] + 63) // 64
            and value["summary_words"] == (value["bitmap_words"] + 63) // 64
            and value["managed_frames"] > 0
            and value["usable_frames"] - value["managed_frames"] == span // 4096
            and details["managed_frames"] == value["managed_frames"]
            and sum((end - start) // 4096 for start, end in areas) == value["managed_frames"]
            and all(end <= value["start"] or start >= value["end"] for start, end in areas)
        )
    checks = {"frame_metadata": valid}
    if not normal:
        checks["metadata_excluded"] = "FRAME: metadata reservation excluded" in log
        for minimum in (1 << 32, 1 << 37):
            if any(max(start, minimum) < end for start, end in details["available"]):
                backing = details["tlb_backing"]
                checks[f"high_tlb_{minimum >> 30}G"] = bool(backing and all(physical >= minimum for physical in backing))
                checks[f"high_frame_{minimum >> 30}G"] = any(
                    item["minimum"] == minimum and item["physical"] >= minimum
                    and any(start <= item["physical"] and item["physical"] + 4096 <= end
                            for start, end in details["available"])
                    for item in details["high_frames"]
                )
    return checks


def resident_memory(pid):
    try:
        status = Path(f"/proc/{pid}/status").read_text()
    except OSError:
        return None
    values = dict(re.findall(r"(?m)^(VmRSS|VmHWM):\s+(\d+) kB$", status))
    if "VmRSS" not in values or "VmHWM" not in values:
        return None
    return {key: int(value) * 1024 for key, value in values.items()}


def observed_checks(log, cpu=None, normal=False, cpus=1):
    if normal:
        supervision = log.partition("BOOT: supervisor dispatched")[2]
        spawned = re.findall(r"(?m)^PROCESS: spawned id=(0x[0-9a-f]+) parent=(0x[0-9a-f]+)$", supervision)
        root = re.search(r"(?m)^BOOT: supervisor dispatched id=(0x[0-9a-f]+) accounts=4$", log)
        ready = re.findall(r"(?m)^USERLOG: process=(0x[0-9a-f]+) event=6 value=2$", supervision)
        persistent = re.findall(r"(?m)^USERLOG: process=(0x[0-9a-f]+) event=7 value=2$", supervision)
        checks = {
            "device": "DEVICE: register authority and IRQ wakeup OK cases=4" in log,
            "native_services": "SERVICE: native IPC and restart OK instances=2 quota=8" in log,
            "smp_online": f"SMP: online={cpus} " in log,
            "normal_supervisor": root is not None,
            "clock": "CLOCK: HPET monotonic ready" in log,
            "manifest_publication": root is not None and len(spawned) == 5
                and len({child for child, _ in spawned}) == 5
                and all(parent == root[1] for _, parent in spawned),
            "authority_and_restart": root is not None and ready == [root[1]],
            "persistent_supervisor": root is not None and persistent.count(root[1]) == 1,
        }
        checks.update(memory_checks(log, normal))
        return checks

    checks = {
        name: bool(re.search(r"(?m)^" + re.escape(marker) + r"(?=\s|$)", log))
        for name, marker in MARKERS.items()
    }
    checks["device"] = "DEVICE: register authority and IRQ wakeup OK cases=4" in log
    checks["smp_online"] = f"SMP: online={cpus} " in log
    checks["smp_retired"] = "SMP: AP resources retired" in log
    checks["smp_tlb"] = "SMP: remote TLB retirement OK" in log
    if cpus > 1:
        checks["smp_parallel"] = "SMP: parallel shared-space OK" in log
        checks["smp_missing_ack"] = "SMP: missing ACK retains backing OK" in log
        checks["smp_wakeup"] = "SMP: blocked thread wakeup OK" in log
        checks["smp_fault_policy"] = "SMP: fatal process policy precedes peer call OK" in log
    checks["breakpoint"] = bool(re.search(r"(?:^|\n)ABC\s*\Z", log))
    if cpu == "max":
        checks["avx_context"] = bool(re.search(r"(?m)^ELF: native execution OK .*xsave=true avx=true$", log))
    checks.update(memory_checks(log, normal))
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


def run_label(profile, cpu=None, normal=False, cpus=1, memory="512M"):
    label = profile + ("-normal" if normal else "")
    label = label if cpu is None else f"{label}-{cpu}"
    label = label if cpus == 1 else f"{label}-smp{cpus}"
    return label if memory == "512M" else f"{label}-ram{memory}"


def run_profile(profile, cpu=None, normal=False, cpus=1, memory="512M", rss_limit_mib=2048):
    memory = parse_memory(memory)
    if rss_limit_mib <= 0:
        raise ValueError("Resident-memory limit must be positive")
    label = run_label(profile, cpu, normal, cpus, memory)
    output = ROOT / "build" / "smoke" / label
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
        "cpu": cpu or "default",
        "cpus": cpus,
        "memory": memory,
        "memory_bytes": memory_bytes(memory),
        "rss_limit_bytes": rss_limit_mib * (1 << 20),
        "peak_rss_bytes": 0,
        "status": "failed",
        "reason": "build not completed",
        "checks": observed_checks("", cpu, normal, cpus),
        "mode": "normal" if normal else "boot-tests",
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
                ["make", "iso", f"BUILD_DIR={output}", f"PROFILE={profile}", "FEATURES=" + ("" if normal else "boot-tests")],
                cwd=ROOT, stdout=stdout, stderr=stderr, check=False,
            )
        evidence["build_exit_code"] = build.returncode
        if build.returncode:
            evidence["reason"] = f"ISO build exited with {build.returncode}"
            return evidence

        logs["debug"].write_text("")
        command = [
            "qemu-system-x86_64", "-M", "q35,memory-backend=ram", "-global", "hpet.msi=on",
            "-object", f"memory-backend-ram,id=ram,size={memory},prealloc=off,reserve=off",
            "-overcommit", "mem-lock=off", "-accel", "tcg,thread=multi", "-smp", str(cpus), "-m", memory,
            "-display", "none", "-serial", "none", "-monitor", "none", "-no-reboot",
            "-debugcon", f"file:{logs['debug']}",
            "-cdrom", str(output / "kryo.iso"), "-boot", "d",
        ]
        if cpu is not None:
            command += ["-cpu", cpu]
        evidence["qemu_command"] = command
        print(f"{profile}: booting QEMU ({BOOT_TIMEOUT}s limit)", flush=True)
        with logs["stdout"].open("w") as stdout, logs["stderr"].open("w") as stderr:
            process = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr)
            evidence["qemu_pid"] = process.pid
            boot_started = time.monotonic()
            while True:
                log = logs["debug"].read_text(errors="replace")
                evidence["checks"] = observed_checks(log, cpu, normal, cpus)
                evidence["memory_observed"] = memory_details(log)
                usage = resident_memory(process.pid)
                if usage is not None:
                    evidence["peak_rss_bytes"] = max(evidence["peak_rss_bytes"], usage["VmRSS"], usage["VmHWM"])
                    if evidence["peak_rss_bytes"] > evidence["rss_limit_bytes"]:
                        evidence["reason"] = f"host resident memory exceeded {rss_limit_mib} MiB"
                        break
                elif process.poll() is None:
                    evidence["reason"] = "cannot measure QEMU resident memory"
                    break
                if normal and any(marker in log for marker in ("BOOT: init terminated", "BOOT: supervisor terminated")):
                    evidence["reason"] = "initial supervisor terminated"
                    break
                fault = FAULT.search(log)
                if fault:
                    evidence["reason"] = f"guest reported {fault[1]}"
                    time.sleep(0.2)
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
    parser.add_argument("--cpu", choices=("max", "qemu64"), help="test a specific QEMU CPU in a separate output directory")
    parser.add_argument("--cpus", type=int, choices=(1, 2, 4, 8, 16, 32, 64, 65, 128, 192, 255), default=1, help="number of guest CPUs")
    parser.add_argument("--memory", type=parse_memory, default="512M", help="guest RAM, for example 128G; demand-backed")
    parser.add_argument("--rss-limit-mib", type=int, default=2048, help="stop own emulator above this measured resident-memory limit")
    parser.add_argument("--normal", action="store_true", help="verify normal init and its user-space restart policy")
    arguments = parser.parse_args()
    profiles = ("debug", "release") if arguments.all else (arguments.profile,)
    passed = True
    for profile in profiles:
        result = run_profile(profile, arguments.cpu, arguments.normal, arguments.cpus, arguments.memory, arguments.rss_limit_mib)
        print(f"{profile}: {result['status']} — {result['reason']}")
        label = run_label(profile, arguments.cpu, arguments.normal, arguments.cpus, arguments.memory)
        print(f"  evidence: {ROOT / 'build' / 'smoke' / label / 'evidence.json'}")
        if result["status"] == "interrupted":
            return 130
        passed &= result["status"] == "passed"
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
