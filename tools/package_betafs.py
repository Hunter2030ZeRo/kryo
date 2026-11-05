#!/usr/bin/env python3
"""Add the optional BetAFS validation programs and explicit volume phase."""

import argparse
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iso-root", type=Path, required=True)
    parser.add_argument("--phase", choices=("format", "open"), required=True)
    arguments = parser.parse_args()
    boot = arguments.iso_root / "boot"
    config = boot / "limine/limine.conf"
    for name in ("betafs-service.elf", "betafs-codecs.elf", "rust-std-probe.elf"):
        if not (boot / name).is_file():
            parser.error(f"Missing native program: {name}")
    phase = b"KRYO-BETAFS-FORMAT-V1\n" if arguments.phase == "format" else b"KRYO-BETAFS-OPEN-V1\n"
    (boot / "betafs-probe.enabled").write_bytes(phase)
    modules = [
        ("rust-std-probe.elf", "kryo-betafs-client"),
        ("betafs-service.elf", "kryo-betafs-service"),
        ("betafs-codecs.elf", "kryo-betafs-codecs"),
        ("betafs-probe.enabled", "kryo-betafs-probe"),
    ]
    config.write_text(config.read_text() + "".join(
        f"\tmodule_path: boot():/boot/{path}\n\tmodule_cmdline: {label}\n" for path, label in modules
    ))


if __name__ == "__main__":
    main()
