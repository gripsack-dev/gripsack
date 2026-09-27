#!/usr/bin/env python3
"""Increment one local counter once per stable activation intent identity."""
import argparse
import json
import os
from pathlib import Path
from counter_store import increment, intent_identity


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state-dir", required=True, type=Path)
    parser.add_argument("--counter", default="example")
    args = parser.parse_args()
    intent = intent_identity(os.environ.get("GRIPSACK_ACTIVATION_INTENT_ID"))
    print(json.dumps(increment(args.state_dir, intent, args.counter), sort_keys=True))


if __name__ == "__main__":
    main()
