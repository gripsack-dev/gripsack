#!/usr/bin/env python3
"""Replace one explicitly selected derived file; repeated delivery is harmless."""
import argparse
import os
from pathlib import Path
import tempfile


def replace(destination: Path, value: bytes) -> None:
    # The caller owns this derived output. This is not an ownership/take-over
    # wrapper for unrelated user files, nor an external-writer CAS.
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(prefix=".hook-replace-", dir=destination.parent,
                                         delete=False) as output:
            temporary = Path(output.name)
            output.write(value)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, destination)
        temporary = None
        parent = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--value", default="configured\n")
    args = parser.parse_args()
    replace(args.destination, args.value.encode())


if __name__ == "__main__":
    main()
