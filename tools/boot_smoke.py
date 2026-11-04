#!/usr/bin/env python3
"""Build and verify one headless QEMU boot without replacing normal boot logs."""

import argparse
import hashlib
import json
import re
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BOOT_TIMEOUT = 90
DEFAULT_CPU = "max"
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
    "resource_budgets": "BUDGET: inheritance, retained owners and recovery OK",
    "private_pending_cleanup": "WAIT: pending thread retirement cancels word",
    "private_expiry_admission": "WAIT: expired word precedes wake admission",
    "private_untimed_idle": "WAIT: unbounded word idle preserves registration",
    "native_thread_rollback": "THREAD: runtime-owned admission rollback and lifetime OK",
}
FAULT = re.compile(r"(?m)^(PANIC|#PF|#GP)(?=\s|$)")


def cpu_checks(log, cpus):
    admitted = re.findall(r"(?m)^CPU: x86-64-v3 hardware admitted cpu=(\d+)$", log)
    return {"cpu_baseline": sorted(map(int, admitted)) == list(range(cpus))}


def kernel_log(log):
    return "\n".join(line for line in log.splitlines() if not line.startswith("USERDIAG: "))


def diagnostic_checks(log):
    records = re.findall(r"(?m)^USERDIAG: process=(0x[0-9a-f]+) bytes=([0-9]+) text=(.*)$", log)
    expected = [b"C native calls ready",
                b"\nPANIC\r\x1b\\\0\x7f\x80ABC\nABI: startup and execution resources retired",
                b"D" * 256, b"Rust diagnostic bridge ready"]
    payloads = []
    for _, length, text in records:
        value = bytearray()
        position = 0
        while position < len(text):
            if text[position:position + 2] == "\\\\":
                value.append(92)
                position += 2
            elif re.match(r"\\x[0-9a-f]{2}", text[position:position + 4]):
                value.append(int(text[position + 2:position + 4], 16))
                position += 4
            elif 0x20 <= ord(text[position]) <= 0x7e and text[position] != "\\":
                value.append(ord(text[position]))
                position += 1
            else:
                return {"native_c_diagnostics": False}
        if len(value) != int(length) or len(value) > 256:
            return {"native_c_diagnostics": False}
        payloads.append(bytes(value))
    return {"native_c_diagnostics": payloads == expected
            and len({record[0] for record in records}) == 1
            and len(records) == log.count("\nUSERDIAG: ") + int(log.startswith("USERDIAG: "))}


def abi_checks(log, enabled):
    if not enabled:
        return {}
    diagnostics = diagnostic_checks(log)
    log = kernel_log(log)
    return {
        "native_abi": bool(re.search(r"(?m)^ABI: v3 SysV C C\+\+ Rust stack and state OK preemptions=[1-9][0-9]*$", log)),
        "native_abi_retired": "ABI: startup and execution resources retired" in log,
        "native_tls": "TLS: ELF template and thread isolation OK threads=3" in log,
        "native_private_vm": "VM: private reserve commit retirement OK" in log,
        "native_atomic_wait": bool(re.search(r"(?m)^WAIT: private atomic park wake retirement OK threads=13 halt=true$", log)),
        "native_runtime_threads": bool(re.search(r"(?m)^THREAD: runtime-owned stack TLS and rounding OK threads=2$", log)),
        "native_c_calls": bool(re.search(r"(?m)^CABI: native C calls and diagnostic retirement OK$", log)),
        "native_c_threads": bool(re.search(r"(?m)^CTHREAD: native C create join and backing retirement OK threads=2$", log)),
        **diagnostics,
    }


def pci_checks(log, attached=False):
    inventory = re.search(
        r"(?m)^PCI: inventory complete functions=(\d+) virtio_block=(\d+) command_unchanged=true$", log)
    functions = re.findall(
        r"(?m)^PCI: function bus=(\d+) slot=(\d+) function=(\d+) vendor=(0x[0-9a-f]+) "
        r"device=(0x[0-9a-f]+) class=(0x[0-9a-f]+) command=(0x[0-9a-f]+) bars=(\d+)$", log)
    addresses = [tuple(map(int, row[:3])) for row in functions]
    blocks = [row for row in functions if int(row[3], 16) == 0x1af4
              and int(row[4], 16) in (0x1001, 0x1042)]
    checks = {"pci_inventory": bool(
        inventory and int(inventory[1]) == len(functions) > 0
        and int(inventory[2]) == len(blocks) and len(set(addresses)) == len(addresses)
        and all(0 <= bus < 256 and 0 <= slot < 32 and 0 <= function < 8
                for bus, slot, function in addresses))}
    if attached:
        checks["pci_storage_behind_bridge"] = len(blocks) == 1 and (
            int(blocks[0][0]) > 0 and int(blocks[0][4], 16) == 0x1042
            and int(blocks[0][5], 16) == 1)
    return checks


def block_checks(log, readonly, normal):
    mode = str(readonly).lower()
    checks = {"block_driver": "BLOCK: Ring3 commands and private DMA retirement OK cases=2" in log}
    for instance in (0, 1):
        checks[f"block_instance_{instance}"] = f"BLOCK: driver complete mode={instance} readonly={mode}" in log
    if not normal:
        checks["block_retirement_retry"] = "BLOCK: cleanup retry retains queue and memory" in log
    return checks


def block_service_checks(log, normal):
    driver = re.search(r"(?m)^BLOCK IPC: service dispatched id=(0x[0-9a-f]+)$", log)
    checks = {
        "block_ipc_cancelled": "BLOCK IPC: accepted cancellation retains loan" in log,
        "block_ipc_waiting": "BLOCK IPC: clients complete; service waiting" in log,
    }
    for event in (91, 92):
        checks[f"block_service_event_{event}"] = bool(driver and
            f"USERLOG: process={driver[1]} event={event} value=1" in log)
    for mode in (0, 1, 3):
        checks[f"block_client_{mode}"] = bool(re.search(rf"(?m)^USERLOG: process=0x[0-9a-f]+ event=93 value={mode}$", log))
    if not normal:
        checks["block_service_retired"] = "BLOCK IPC: graceful stop and resource retirement OK" in log
    return checks


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


def observed_checks(log, cpu=None, normal=False, cpus=1, pci_probe=False, block_probe=False, block_readonly=False, abi_probe=False):
    raw_log = log
    log = kernel_log(log)
    budget_stages = {int(value) for value in re.findall(r"(?m)^USERLOG: process=0x[0-9a-f]+ event=12 value=(\d+)$", log)}
    budget_clients = all((service << 32) | stage in budget_stages for service in (20, 21) for stage in (1, 2, 3))
    if normal:
        supervision = log.partition("BOOT: supervisor dispatched")[2]
        spawned = re.findall(r"(?m)^PROCESS: spawned id=(0x[0-9a-f]+) parent=(0x[0-9a-f]+)$", supervision)
        root = re.search(r"(?m)^BOOT: supervisor dispatched id=(0x[0-9a-f]+) accounts=4 budgets=4$", log)
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
            "client_budget_exhaustion": budget_clients,
        }
        checks.update(memory_checks(log, normal))
        checks.update(cpu_checks(log, cpus))
        checks.update(abi_checks(raw_log, abi_probe))
        checks.update(pci_checks(log, pci_probe or block_probe))
        if block_probe:
            checks.update(block_checks(log, block_readonly, normal))
            checks.update(block_service_checks(log, normal))
        checks["dma_buffer_ownership"] = "DMA: unpublished buffer zeroing and retirement OK pages=3" in log
        return checks

    checks = {
        name: bool(re.search(r"(?m)^" + re.escape(marker) + r"(?=\s|$)", log))
        for name, marker in MARKERS.items()
    }
    checks["device"] = "DEVICE: register authority and IRQ wakeup OK cases=4" in log
    checks["client_budget_exhaustion"] = budget_clients
    checks["smp_online"] = f"SMP: online={cpus} " in log
    checks["smp_retired"] = "SMP: AP resources retired" in log
    checks["smp_tlb"] = "SMP: remote TLB retirement OK" in log
    if cpus > 1:
        checks["smp_parallel"] = "SMP: parallel shared-space OK" in log
        checks["smp_missing_ack"] = "SMP: missing ACK retains backing OK" in log
        checks["smp_wakeup"] = "SMP: blocked thread wakeup OK" in log
        checks["smp_fault_policy"] = "SMP: fatal process policy precedes peer call OK" in log
    checks["breakpoint"] = bool(re.search(r"(?:^|\n)ABC\s*\Z", log))
    checks["avx_context"] = bool(re.search(r"(?m)^ELF: native execution OK .*xsave=true avx=true$", log))
    checks.update(cpu_checks(log, cpus))
    checks.update(abi_checks(raw_log, abi_probe))
    checks.update(memory_checks(log, normal))
    checks.update(pci_checks(log, pci_probe or block_probe))
    if block_probe:
        checks.update(block_checks(log, block_readonly, normal))
        checks.update(block_service_checks(log, normal))
    checks["dma_buffer_ownership"] = "DMA: unpublished buffer zeroing and retirement OK pages=3" in log
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


def run_label(profile, cpu=None, normal=False, cpus=1, memory="512M", pci_probe=False, block_probe=False, block_readonly=False, abi_probe=False):
    label = profile + ("-normal" if normal else "")
    label = label if cpu is None else f"{label}-{cpu}"
    label = label if cpus == 1 else f"{label}-smp{cpus}"
    label = label if memory == "512M" else f"{label}-ram{memory}"
    label += "-abi" if abi_probe else ""
    return label + ("-block-ro" if block_readonly else "-block-rw" if block_probe else "-pci" if pci_probe else "")


def run_profile(profile, cpu=None, normal=False, cpus=1, memory="512M", rss_limit_mib=2048, pci_probe=False, block_probe=False, block_readonly=False, abi_probe=False):
    memory = parse_memory(memory)
    if rss_limit_mib <= 0:
        raise ValueError("Resident-memory limit must be positive")
    if block_readonly and not block_probe:
        raise ValueError("Read-only block mode requires the block probe")
    label = run_label(profile, cpu, normal, cpus, memory, pci_probe, block_probe, block_readonly, abi_probe)
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
        "cpu": cpu or DEFAULT_CPU,
        "cpus": cpus,
        "memory": memory,
        "pci_probe": pci_probe,
        "block_probe": block_probe,
        "block_readonly": block_readonly,
        "abi_probe": abi_probe,
        "memory_bytes": memory_bytes(memory),
        "rss_limit_bytes": rss_limit_mib * (1 << 20),
        "peak_rss_bytes": 0,
        "status": "failed",
        "reason": "build not completed",
        "checks": observed_checks("", cpu, normal, cpus, pci_probe, block_probe, block_readonly, abi_probe),
        "mode": "normal" if normal else "boot-tests",
        "logs": {name: str(path) for name, path in logs.items()},
        "guest_exit_verified": False,
        "scope": "QEMU TCG boot markers; host termination does not prove guest exit",
    }
    process = None
    disk = None
    boot_started = None
    started = time.monotonic()

    try:
        print(f"{profile}: building ISO", flush=True)
        with logs["build_stdout"].open("w") as stdout, logs["build_stderr"].open("w") as stderr:
            build = subprocess.run(
                ["make", "iso", f"BUILD_DIR={output}", f"CARGO_TARGET_DIR={output / 'kernel-target'}", f"PROFILE={profile}", "FEATURES=" + ("" if normal else "boot-tests"), f"BLOCK_PROBE={int(block_probe)}", f"ABI_PROBE={int(abi_probe)}"],
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
        command += ["-cpu", cpu or DEFAULT_CPU]
        if pci_probe or block_probe:
            # Every attached disk is disposable and has no filesystem. Block
            # tests opt in to writes; discovery-only fixtures stay read-only.
            disk = output / ("block-probe.raw" if block_probe else "pci-probe.raw")
            with disk.open("wb") as image:
                image.truncate(8 << 20)
                if block_probe:
                    image.write(b"CRYOLITE-BLOCK-PROBE-V1")
            evidence["probe_disk_sha256"] = hashlib.sha256(disk.read_bytes()).hexdigest()
            expected = bytearray(disk.read_bytes())
            if block_probe and not block_readonly:
                expected[4096:8192] = bytes((offset * 37 + 11) & 255 for offset in range(4096))
                expected[8192:12288] = bytes((offset * 17 + 0x53) & 255 for offset in range(4096))
            evidence["expected_disk_sha256"] = hashlib.sha256(expected).hexdigest()
            read_only = not block_probe or block_readonly
            command += [
                "-device", "pcie-root-port,id=storage-port,bus=pcie.0,chassis=1,slot=5",
                "-drive", f"file={disk},format=raw,if=none,id=probe-disk,readonly={'on' if read_only else 'off'},cache=writeback",
                "-device", "virtio-blk-pci,drive=probe-disk,disable-legacy=on,bus=storage-port",
            ]
        evidence["qemu_command"] = command
        print(f"{profile}: booting QEMU ({BOOT_TIMEOUT}s limit)", flush=True)
        with logs["stdout"].open("w") as stdout, logs["stderr"].open("w") as stderr:
            process = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr)
            evidence["qemu_pid"] = process.pid
            boot_started = time.monotonic()
            while True:
                log = logs["debug"].read_text(errors="replace")
                evidence["checks"] = observed_checks(log, cpu, normal, cpus, pci_probe, block_probe, block_readonly, abi_probe)
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
        if disk is not None and "expected_disk_sha256" in evidence:
            try:
                digest = hashlib.sha256(disk.read_bytes()).hexdigest()
                unchanged = digest == evidence["probe_disk_sha256"]
                verified = digest == evidence["expected_disk_sha256"]
            except OSError:
                unchanged = False
                verified = False
            evidence["probe_disk_unchanged"] = unchanged
            evidence["probe_disk_verified"] = verified
            if not verified:
                evidence["status"] = "failed"
                evidence["reason"] = "disposable block fixture could not be verified"
        (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")

    return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--profile", choices=("debug", "release"), default="debug")
    selection.add_argument("--all", action="store_true", help="test debug and release once each")
    parser.add_argument("--cpu", choices=("max", "Haswell-noTSX", "qemu64"), help="QEMU CPU; default: max (x86-64-v3 required; qemu64 is unsupported)")
    parser.add_argument("--cpus", type=int, choices=(1, 2, 4, 8, 16, 32, 64, 65, 128, 192, 255), default=1, help="number of guest CPUs")
    parser.add_argument("--memory", type=parse_memory, default="512M", help="guest RAM, for example 128G; demand-backed")
    parser.add_argument("--rss-limit-mib", type=int, default=2048, help="stop own emulator above this measured resident-memory limit")
    parser.add_argument("--normal", action="store_true", help="verify normal init and its user-space restart policy")
    parser.add_argument("--abi-probe", action="store_true", help="build and run the freestanding v3 C/C++/Rust SysV startup and state probe")
    parser.add_argument("--pci-probe", action="store_true", help="attach a disposable read-only VirtIO block device behind a PCI bridge; discovery only")
    parser.add_argument("--block-probe", action="store_true", help="opt in to Ring 3 block read/write/flush on a disposable signed disk")
    parser.add_argument("--block-readonly", action="store_true", help="test device-rejected writes; requires --block-probe")
    arguments = parser.parse_args()
    if arguments.block_readonly and not arguments.block_probe:
        parser.error("--block-readonly requires --block-probe")
    if arguments.pci_probe and arguments.block_probe:
        parser.error("choose discovery-only or block probe")
    profiles = ("debug", "release") if arguments.all else (arguments.profile,)
    passed = True
    for profile in profiles:
        result = run_profile(profile, arguments.cpu, arguments.normal, arguments.cpus, arguments.memory, arguments.rss_limit_mib, arguments.pci_probe, arguments.block_probe, arguments.block_readonly, arguments.abi_probe)
        print(f"{profile}: {result['status']} — {result['reason']}")
        label = run_label(profile, arguments.cpu, arguments.normal, arguments.cpus, arguments.memory, arguments.pci_probe, arguments.block_probe, arguments.block_readonly, arguments.abi_probe)
        print(f"  evidence: {ROOT / 'build' / 'smoke' / label / 'evidence.json'}")
        if result["status"] == "interrupted":
            return 130
        passed &= result["status"] == "passed"
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
