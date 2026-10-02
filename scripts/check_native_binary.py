#!/usr/bin/env python3
"""Reject dynamic Linux dependencies and non-system macOS dependencies."""
import argparse
from pathlib import Path
import platform
import subprocess

LIBRARIES = {"libSystem.B.dylib", "libiconv.2.dylib", "libobjc.A.dylib", "libc++.1.dylib"}
FRAMEWORKS = {"CoreFoundation", "Security", "SystemConfiguration", "Foundation", "Virtualization"}


def verify_macho(binary, listing):
    allowed = {f"/usr/lib/{name}" for name in LIBRARIES}
    allowed.update(f"/System/Library/Frameworks/{name}.framework/Versions/{version}/{name}"
                   for name in FRAMEWORKS for version in ("A", "C"))
    lines = listing.splitlines()
    if not lines or lines[0] != f"{binary}:":
        raise SystemExit("otool output missing exact binary header")
    dependencies = []
    for line in lines[1:]:
        dependency, marker, versions = line.strip().partition(" (compatibility version ")
        if not marker or not versions.endswith(")") or dependency not in allowed:
            raise SystemExit(f"unapproved Mach-O dependency: {line}")
        dependencies.append(dependency)
    if "/usr/lib/libSystem.B.dylib" not in dependencies:
        raise SystemExit("Mach-O dependency listing missing libSystem")


def verify_elf(headers, dynamic):
    if "INTERP" in headers or "(NEEDED)" in dynamic:
        raise SystemExit("Linux binary must be static, without an interpreter or shared libraries")
    if "LOAD" not in headers:
        raise SystemExit("ELF program headers missing load segments")


def verify(binary):
    if platform.system() == "Darwin":
        verify_macho(binary, subprocess.check_output(["otool", "-L", str(binary)], text=True))
    elif platform.system() == "Linux":
        verify_elf(subprocess.check_output(["readelf", "-l", str(binary)], text=True),
                   subprocess.check_output(["readelf", "-d", str(binary)], text=True))
    else:
        raise SystemExit("native dependency verification requires Linux or macOS")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    verify(parser.parse_args().binary)
