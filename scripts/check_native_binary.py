#!/usr/bin/env python3
"""Reject dynamic dependencies in shipped Linux binaries."""
import argparse
from pathlib import Path
import platform
import subprocess



def verify_elf(headers, dynamic):
    if "INTERP" in headers or "(NEEDED)" in dynamic:
        raise SystemExit("Linux binary must be static, without an interpreter or shared libraries")
    if "LOAD" not in headers:
        raise SystemExit("ELF program headers missing load segments")


def verify(binary):
    if platform.system() != "Linux":
        raise SystemExit("native dependency verification requires Linux")
    verify_elf(subprocess.check_output(["readelf", "-l", str(binary)], text=True),
               subprocess.check_output(["readelf", "-d", str(binary)], text=True))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    verify(parser.parse_args().binary)
