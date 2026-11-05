#!/usr/bin/env python3
"""Check experimental C profile headers without using host libc headers."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
PROFILE = ROOT / "userspace/libc/abi"
NATIVE = ROOT / "userspace/libsys/include/cryolite/native.h"
NAME = re.compile(r"[A-Z][A-Z0-9_]*\Z")
NUMBER = re.compile(r"-?(?:0x[0-9a-fA-F]+|[0-9]+)\Z")


def table_constants(path):
    values = {}
    for line in path.read_text().splitlines():
        if not line.startswith("|"):
            continue
        cells = [cell.strip().strip("`") for cell in line.strip("|").split("|")]
        for index in range(0, len(cells) - 1, 2):
            name, value = cells[index].split(",", 1)[0], cells[index + 1]
            if NAME.fullmatch(name) and (NUMBER.fullmatch(value) or NAME.fullmatch(value)):
                if name in values:
                    raise ValueError(f"Repeated constant {name} in {path}")
                values[name] = value
    return values


def enum_values(source, declaration):
    body = re.search(re.escape(declaration) + r"\s*\{([^}]+)\}", source)
    if not body:
        raise ValueError(f"Missing enum {declaration}")
    return {name: int(value.replace("_", ""), 0)
            for name, value in re.findall(r"(\w+)\s*=\s*(-?[0-9][0-9_]*)", body[1])}


def rust_constant(source, name):
    value = re.search(r"\bconst " + name + r"\s*:[^=;]+=[ \n]*(0x[0-9a-fA-F_]+|[0-9_]+)\s*;", source)
    if not value:
        raise ValueError(f"Missing numeric Rust constant {name}")
    return int(value[1].replace("_", ""), 0)


def screaming(name):
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).upper()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--outputdir", "--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.outputdir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    pin_path = ROOT / "targets/cryolite-toolchain.json"
    target_path = ROOT / "targets/x86_64-unknown-cryolite.json"
    pin, target = json.loads(pin_path.read_text()), json.loads(target_path.read_text())
    assert target["cpu"] == "x86-64-v3" and "-mmx" in target["features"].split(",")
    versions = {cc: subprocess.check_output(["rtk", "proxy", cc, "--version"], text=True)
                for cc in ("clang", "clang++")}
    assert all(v.splitlines()[0] == f"clang version {pin['clang_version']}" for v in versions.values())
    versions["rustc"] = subprocess.check_output(["rtk", "proxy", "rustc", "--version", "--verbose"],
                                               cwd=ROOT, text=True)
    assert f"commit-hash: {pin['rustc_commit']}\n" in versions["rustc"]
    checks = []

    def run(name, command, rejection=None, scope="target compile", directory=ROOT):
        result = subprocess.run(command, cwd=directory, text=True, capture_output=True)
        (output / f"{name}.stdout.log").write_text(result.stdout)
        (output / f"{name}.stderr.log").write_text(result.stderr)
        passed = result.returncode == 0 if rejection is None else (
            result.returncode != 0 and rejection in result.stderr)
        checks.append({"name": name, "command": command, "directory": str(directory), "scope": scope,
                       "exit_code": result.returncode, "expected_rejection": rejection, "passed": passed})
        if not passed:
            raise RuntimeError(f"{name} failed: {result.stderr}")

    flags = [f"--target={target['llvm-target']}", "-march=x86-64-v3", "-mno-mmx", "-nostdinc",
             "-ffreestanding", "-fno-stack-protector", "-Wall", "-Wextra", "-Werror", "-pedantic-errors",
             "-I", str(PROFILE / "include"), "-I", str(NATIVE.parents[1])]
    headers = ("types", "errno", "memory", "time", "file", "signal")
    layout = PROFILE / "tests/layout.c"
    thread_fixture = ROOT / "userspace/libsys/c/tests/thread.c"
    ipc_fixture = ROOT / "userspace/libsys/c/tests/ipc.c"
    for compiler, standard, language in (("clang", "c11", "c"), ("clang++", "c++23", "c++")):
        base = ["rtk", "proxy", compiler, *flags, f"-std={standard}", "-x", language]
        if language == "c++":
            base += ["-fno-exceptions", "-fno-rtti"]
        for header in headers:
            path = output / f"individual-{header}.c"
            path.write_text(f"#include <cryolite/{header}.h>\n#include <cryolite/{header}.h>\nint header_check;\n")
            run(f"individual-{header}-{language}", [*base, "-fsyntax-only", str(path)])
        for label, order in (("normal", (*headers, "native")), ("reverse", ("native", *reversed(headers)))):
            path = output / f"layout-{label}.c"
            path.write_text("".join(f"#include <cryolite/{name}.h>\n" for name in order)
                            + f'#include "{layout}"\n')
            run(f"layout-{label}-{language}", [*base, "-c", str(path), "-o", str(output / f"{label}-{language}.o")])
        for optimization in ("-O0", "-O2"):
            for mode, options in (("static", ("-fno-pic", "-fno-pie")),
                                  ("pic-frame", ("-fPIC", "-fno-omit-frame-pointer"))):
                name = f"thread-{language}-{optimization[1:]}-{mode}"
                run(name, [*base, optimization, *options, "-mno-red-zone",
                           "-c", str(thread_fixture), "-o", str(output / f"{name}.o")],
                    scope="Target SDK thread layout and compiler/register constraints; no execution")
                name = f"ipc-{language}-{optimization[1:]}-{mode}"
                artifact = output / f"{name}.o"
                run(name, [*base, optimization, *options, "-mno-red-zone",
                           "-c", str(ipc_fixture), "-o", str(artifact)],
                    scope="Target SDK IPC/capability/shared-memory layout and register constraints; no execution")
                audit = ["rtk", "proxy", "llvm-nm", "--undefined-only", str(artifact)]
                symbols = subprocess.check_output(audit, text=True)
                (output / f"{name}.undefined.log").write_text(symbols)
                assert not symbols.strip(), (name, symbols)
                checks[-1]["undefined_symbol_audit"] = audit
                checks[-1]["undefined_symbols"] = []

    tables = {name: table_constants(ROOT / f"docs/abi/c-v1-{name}.md")
              for name in ("errors", "memory-time", "files", "signals")}
    assert {name: len(values) for name, values in tables.items()} == {
        "errors": 81, "memory-time": 25, "files": 96, "signals": 46}
    constants = {}
    for values in tables.values():
        for name, value in values.items():
            assert name not in constants
            constants[name] = value
    syscall_path, startup_path = ROOT / "interfaces/src/syscall/mod.rs", ROOT / "interfaces/src/startup.rs"
    libsys_path = ROOT / "userspace/libsys/src/lib.rs"
    syscall, startup, libsys, native = (path.read_text() for path in (syscall_path, startup_path, libsys_path, NATIVE))
    calls = {"KRYO_CALL_" + screaming(name): value for name, value in enum_values(syscall, "pub enum Call").items()}
    exposed = enum_values(native, "enum kryo_call")
    assert len(exposed) == 48 and exposed.keys() <= calls.keys()
    native_expected = {name: calls[name] for name in exposed}
    statuses = {"KRYO_STATUS_" + screaming(name): -value for name, value in enum_values(syscall, "pub enum Error").items()}
    statuses["KRYO_STATUS_OK"] = 0
    assert enum_values(native, "enum kryo_status_code").keys() == statuses.keys()
    native_expected.update(statuses)
    wait = re.search(r"fn wait\([\s\S]*?\n}\n", libsys)[0]
    exit_reasons = {"KRYO_EXIT_" + screaming(name): int(value)
                    for value, name in re.findall(r"(\d+)(?:u64)? => ExitReason::(\w+)", wait)}
    assert len(exit_reasons) == 3
    assert enum_values(native, "enum kryo_exit_reason").keys() == exit_reasons.keys()
    native_expected.update(exit_reasons)
    native_expected.update({"KRYO_NATIVE_ABI_VERSION": rust_constant(syscall, "ABI_VERSION"),
                            "KRYO_NATIVE_THREAD_VERSION": rust_constant(syscall, "NATIVE_THREAD_VERSION"),
                            "KRYO_DEBUG_WRITE_MAX": rust_constant(syscall, "DEBUG_WRITE_MAX"),
                            "KRYO_RANDOM_WORDS": rust_constant(syscall, "RANDOM_WORDS")})
    for name in ("INSPECT", "CONTROL", "DUP", "SEND", "RECEIVE", "WAIT", "SIGNAL", "READ",
                 "WRITE", "MAP", "TRANSFER", "EXECUTE", "CREATE", "ASSIGN"):
        native_expected["KRYO_RIGHT_" + name] = rust_constant(libsys, name)
    for name, value in enum_values(startup, "pub enum ObjectKind").items():
        native_expected["KRYO_OBJECT_" + screaming(name)] = value
    events = enum_values(syscall, "pub enum ReceiveEvent")
    assert events == {"Message": 0, "GateClosed": 1}
    for name, value in events.items():
        native_expected["KRYO_EVENT_" + screaming(name)] = value
    native_expected["KRYO_IPC_WORDS"] = int(re.search(r"pub type Message = \[u64;\s*(\d+)\]", libsys)[1])
    received = re.search(r"pub struct Received\s*\{([^}]+)\}", libsys)[1]
    native_expected["KRYO_IPC_CAPS"] = int(re.search(r"handles: \[Option<Handle>;\s*(\d+)\]", received)[1])
    native_expected["KRYO_MEMORY_READ_ONLY"] = 0
    native_expected["KRYO_MEMORY_READ_WRITE"] = 1
    for name in ("VERSION", "HEADER_WORDS", "ENTRY_WORDS", "MAX_GRANTS"):
        native_expected["KRYO_STARTUP_" + name] = rust_constant(startup, name)
    magic = re.search(r'pub const MAGIC: u64 = u64::from_le_bytes\(\*b"([A-Z]+)"\);', startup)
    assert magic and len(magic[1]) == 8
    native_expected["KRYO_STARTUP_MAGIC"] = int.from_bytes(magic[1].encode("ascii"), "little")
    park = re.search(r"fn park_result\([^}]+", libsys)[0]
    native_expected["KRYO_PARK_WOKEN"] = int(re.search(r"(\d+) => Ok\(ParkResult::Woken\)", park)[1])
    native_expected["KRYO_PARK_CHANGED"] = int(re.search(r"(\d+) => Ok\(ParkResult::ValueChanged\)", park)[1])
    generated = [f'#include "{layout}"']
    for name, value in {**constants, **native_expected}.items():
        generated.append(f"CHECK({name} == {value});")
        if name in constants and name not in ("P_ALL", "P_PID", "P_PGID"):
            generated += [f"#if {name} != {value}", f"#error {name}", "#endif"]
    constant_source = output / "constants.c"
    constant_source.write_text("\n".join(generated) + "\n")
    for cc, std, lang in (("clang", "c11", "c"), ("clang++", "c++23", "c++")):
        extra = ["-fno-exceptions", "-fno-rtti"] if lang == "c++" else []
        run(f"constants-{lang}", ["rtk", "proxy", cc, *flags, f"-std={std}", "-x", lang,
                                 *extra, "-c", str(constant_source), "-o", str(output / f"constants-{lang}.o")])
    for label, option, diagnostic in (("long-double-64", "-mlong-double-64", "requires the selected 80-bit"),
                                      ("long-double-128", "-mlong-double-128", "requires the selected 80-bit"),
                                      ("unsigned-char", "-funsigned-char", "requires signed plain char"),
                                      ("packed", "-fpack-struct=1", "static assertion failed")):
        run("reject-" + label, ["rtk", "proxy", "clang", *flags, "-std=c11", option,
                                "-fsyntax-only", str(layout)], diagnostic)
    host = output / "wait-host"
    run("wait-host-build", ["rtk", "proxy", "clang", "--target=x86_64-unknown-linux-gnu",
                            "-march=x86-64-v3", "-mno-mmx", "-nostdinc", "-I", str(PROFILE / "include"),
                            "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror",
                            str(PROFILE / "tests/wait.c"), "-o", str(host)], scope="Linux host macro-only build")
    run("wait-host-run", ["rtk", "proxy", "./wait-host"],
        scope="Linux host macro semantics; no native calls", directory=output)

    ipc_host = output / "ipc-host"
    run("ipc-host-build", ["rtk", "proxy", "clang", "--target=x86_64-unknown-linux-gnu",
                           "-march=x86-64-v3", "-mno-mmx", "-nostdinc", "-I", str(NATIVE.parents[1]),
                           "-std=c11", "-O2", "-ffunction-sections", "-Wl,--gc-sections",
                           "-Wall", "-Wextra", "-Werror", "-DKRYO_HOST_CHECK",
                           str(ipc_fixture), "-o", str(ipc_host)],
        scope="Linux host IPC argument/accepted-state helpers; no admitted native calls")
    run("ipc-host-disassembly", ["rtk", "proxy", "objdump", "-d", "-M", "intel", str(ipc_host)],
        scope="Reject native int80 instructions before running host-only validation")
    assembly = (output / "ipc-host-disassembly.stdout.log").read_text()
    assert not re.search(r"\bint\s+(?:0x80|128)\b", assembly), "Host IPC test contains a native entry"
    run("ipc-host-run", ["rtk", "proxy", "./ipc-host"],
        scope="Linux host rejected arguments and accepted-state semantics; no native calls", directory=output)

    inputs = [Path(__file__).resolve(), pin_path, target_path, layout, thread_fixture, ipc_fixture, PROFILE / "tests/wait.c", NATIVE,
              syscall_path, startup_path, libsys_path]
    inputs += sorted((PROFILE / "include/cryolite").glob("*.h"))
    inputs += [ROOT / f"docs/abi/c-v1-{name}.md" for name in tables]
    inputs += [ROOT / f"docs/validation/2026-11-04-c-abi-registry/{name}.h" for name in ("files", "signals")]
    report = {"scope": "Target C/C++ profile and SDK header compilation; Linux host macro semantics only; no installed sysroot, libc build or new Cryolite execution",
              "versions": versions, "checks": checks, "all_passed": all(item["passed"] for item in checks),
              "table_constants": tables, "native_constants": native_expected, "host_wait_statuses": 65536,
              "host_single_evaluation_cases": 4,
              "inputs": {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in inputs}}
    (output / "evidence.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"Passed {len(checks)} checks; {len(constants)} profile and {len(native_expected)} native constants")


if __name__ == "__main__":
    main()
