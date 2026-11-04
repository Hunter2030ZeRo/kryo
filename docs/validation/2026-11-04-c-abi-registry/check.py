#!/usr/bin/env python3
"""Check candidate layouts, constant tables and v3 code generation; not timing."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
NAME = re.compile(r"[A-Z][A-Z0-9_]*\Z")
NUMBER = re.compile(r"-?(?:0x[0-9a-fA-F]+|[0-9]+)\Z")


def constants(path):
    values = {}
    for line in path.read_text().splitlines():
        if not line.startswith("|"):
            continue
        cells = [cell.strip().strip("`") for cell in line.strip("|").split("|")]
        for index in range(0, len(cells) - 1, 2):
            name = cells[index].split(",", 1)[0]
            value = cells[index + 1]
            if NAME.fullmatch(name) and (NUMBER.fullmatch(value) or NAME.fullmatch(value)):
                if name in values:
                    raise ValueError(f"Repeated constant {name} in {path}")
                values[name] = value
    return values


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    profile = json.loads((ROOT / "targets/x86_64-unknown-cryolite.json").read_text())
    pin = json.loads((ROOT / "targets/cryolite-toolchain.json").read_text())
    assert profile["cpu"] == "x86-64-v3"
    assert "-mmx" in profile["features"].split(",")
    assert '"-mno-mmx"' in (ROOT / "userspace/abi-probe/build.rs").read_text()
    versions = {name: subprocess.check_output([name, "--version"], text=True)
                for name in ("clang", "clang++")}
    versions["rustc"] = subprocess.check_output(
        ["rustc", "--version", "--verbose"], cwd=ROOT, text=True)
    assert all(versions[name].splitlines()[0] == f"clang version {pin['clang_version']}"
               for name in ("clang", "clang++"))
    assert f"commit-hash: {pin['rustc_commit']}\n" in versions["rustc"]

    tables = {name: constants(ROOT / f"docs/abi/c-v1-{name}.md")
              for name in ("errors", "memory-time", "files", "signals")}
    primary = {key: int(value) for key, value in tables["errors"].items() if value.isdigit()}
    assert len(primary) == 79 and list(primary) == sorted(primary)
    assert list(primary.values()) == list(range(1, 80))
    assert tables["errors"]["EWOULDBLOCK"] == "EAGAIN"
    assert tables["errors"]["EOPNOTSUPP"] == "ENOTSUP"
    assert len(tables["errors"]) == 81
    source = ['#include "files.h"', '#include "signals.h"']
    for group, values in tables.items():
        for name, value in values.items():
            prefix = "DRAFT_" if group in ("files", "signals") else "ABI_"
            expected = prefix + value if NAME.fullmatch(value) else value
            if prefix == "ABI_":
                source.append(f"#define {prefix}{name} {expected}")
            source.append(f'_Static_assert({prefix}{name} == {expected}, "{name}");')
            if name not in ("P_ALL", "P_PID", "P_PGID"):
                source += [f"#if {prefix}{name} != {expected}", f"#error {name}", "#endif"]
    generated = output / "constants.c"
    generated.write_text("\n".join(source) + "\n")

    checks = []

    def run(name, command, rejection=None):
        result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
        (output / f"{name}.stdout.log").write_text(result.stdout)
        (output / f"{name}.stderr.log").write_text(result.stderr)
        passed = (result.returncode == 0 if rejection is None else
                  result.returncode != 0 and rejection in result.stderr)
        checks.append({"name": name, "command": command, "exit_code": result.returncode,
                       "expected_rejection": rejection, "passed": passed})
        if not passed:
            raise RuntimeError(f"{name} failed: {result.stderr}")

    flags = [f"--target={profile['llvm-target']}", "-march=x86-64-v3", "-mno-mmx",
             "-ffreestanding", "-Wall", "-Wextra", "-Werror"]
    scalar = ROOT / "docs/validation/2026-11-03-c-abi-design/layout.c"
    for compiler, standard, language in (("clang", "c11", "c"), ("clang++", "c++23", "c++")):
        label = "c" if language == "c" else "cpp"
        for name, path in (("scalar", scalar), ("layout", HERE / "layout.c")):
            run(f"{name}-{label}", [compiler, *flags, f"-std={standard}", "-x", language,
                                     "-c", str(path), "-o", str(output / f"{name}-{label}.o")])
    run("constants", ["clang", *flags, "-std=c11", "-I", str(HERE), "-c",
                      str(generated), "-o", str(output / "constants.o")])
    run("rust-host", ["rustc", "--edition=2024", "--crate-type=lib", "--emit=obj",
                      "--target", "x86_64-unknown-linux-gnu", "-Dwarnings",
                      str(HERE / "layout.rs"), "-o", str(output / "layout-rust.o")])
    assembly = output / "isa.s"
    run("isa", ["clang", *flags, "-O2", "-ffp-contract=off", "-S", "-masm=intel",
                str(HERE / "isa.c"), "-o", str(assembly)])
    code = assembly.read_text()
    assert not re.search(r"\b(?:mm[0-7]|emms|femms)\b", code)
    for opcode in (r"vaddsd", r"vpaddd", r"vfmadd\d+sd", r"lzcnt", r"bzhi", r"fld", r"faddp"):
        assert re.search(r"\b" + opcode + r"\b", code), opcode
    run("reject-mmx", ["clang", *flags, "-mmmx", "-fsyntax-only", str(HERE / "isa.c")],
        "Native code generation must disable MMX")
    run("reject-v2", ["clang", *flags, "-march=x86-64-v2", "-fsyntax-only", str(HERE / "isa.c")],
        "native compiler profile must expose")
    for width in (64, 128):
        run(f"reject-long-double-{width}",
            ["clang", *flags, f"-mlong-double-{width}", "-fsyntax-only", str(scalar)],
            "static assertion failed")

    inputs = [scalar, HERE / "layout.c", HERE / "layout.rs", HERE / "isa.c",
              HERE / "files.h", HERE / "signals.h", Path(__file__).resolve(),
              ROOT / "targets/x86_64-unknown-cryolite.json",
              ROOT / "targets/cryolite-toolchain.json", ROOT / "userspace/abi-probe/build.rs"]
    inputs += [ROOT / f"docs/abi/c-v1-{name}.md" for name in tables]
    report = {"scope": "Target C/C++ and Rust host compile checks, registry consistency and ISA availability; no libc build, signal delivery or hardware timing",
              "versions": versions, "constants": tables, "checks": checks,
              "all_passed": all(check["passed"] for check in checks),
              "inputs": {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                         for path in inputs}}
    (output / "compile-evidence.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"Passed {len(checks)} compiler checks; constants per table: "
          + ", ".join(f"{name}={len(values)}" for name, values in tables.items()))


if __name__ == "__main__":
    main()
