#!/usr/bin/env python3
"""Compare Docker's loaded image with independently verified OCI evidence."""
from __future__ import annotations

import json
from pathlib import Path
import sys


def check(verified_path: Path, inspect_path: Path) -> None:
    verified = json.loads(verified_path.read_text())
    inspected = json.loads(inspect_path.read_text())
    if not isinstance(inspected, list) or len(inspected) != 1:
        sys.exit("Docker inspect must report exactly one loaded image")
    image = inspected[0]
    original = verified["first"]
    archive = verified["docker-archive"]
    if archive["config-digest"] != original["config-digest"]:
        sys.exit("Docker archive did not bind the verified OCI config")
    if archive["layer-diff-ids"] != original["diff-ids"]:
        sys.exit("Docker archive did not bind the verified OCI layer DiffIDs")
    if archive["tag"] not in image.get("RepoTags", []):
        sys.exit("Docker loaded a different image tag")
    if f"{image.get('Os')}/{image.get('Architecture')}" != original["platform"]:
        sys.exit("Docker loaded an image for a different platform")
    if image.get("RootFS", {}).get("Layers") != original["diff-ids"]:
        sys.exit("Docker loaded layer bytes different from verified OCI DiffIDs")

    # B0's minimal fixture admits only these runtime config fields. A
    # future field needs an explicit independent comparison, not a
    # 'best effort' parse or an assumed Docker-preserved config hash.
    config = original["runtime-config"]
    if not isinstance(config, dict) or set(config) != {"Env", "WorkingDir"}:
        sys.exit("unqualified OCI runtime config fields")
    loaded = image.get("Config") or {}
    for key, value in config.items():
        if loaded.get(key) != value:
            sys.exit(f"Docker loaded different {key} from verified OCI config")
    for key in ("Cmd", "Entrypoint", "User", "Labels", "ExposedPorts", "Volumes", "Healthcheck"):
        if loaded.get(key) not in (None, "", {}, []):
            sys.exit(f"Docker added unexpected executable runtime field {key}")
    print(
        f"runtime-image: verified OCI platform/config/layer DiffIDs and tag match "
        f"Docker image {image['Id']} (Docker may re-encode config metadata)"
    )


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: check_loaded.py VERIFIED_OCI_REPORT.json DOCKER_INSPECT.json")
    check(Path(sys.argv[1]), Path(sys.argv[2]))
