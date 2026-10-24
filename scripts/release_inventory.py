"""Read-only source package inventory: no Cargo invocation or build-script execution."""
from __future__ import annotations

import os
from pathlib import Path
import re
import tomllib

from release_manifest import MAX_JSON_BYTES, VERSION, load_json, regular, require


def source_inventory(source):
    source = Path(source).absolute()

    def manifest(path):
        path = regular(path)
        require(path.stat().st_size <= MAX_JSON_BYTES, "Cargo manifest exceeds bound")
        return tomllib.loads(path.read_text())

    root = manifest(source / "Cargo.toml")
    workspace = root["workspace"]
    require(not any(key in root for key in ("patch", "replace")), "unsupported Cargo source overrides")
    require(not workspace.get("exclude"), "unsupported Cargo workspace exclusions")
    members = workspace["members"]
    require(isinstance(members, list) and members, "missing Cargo workspace members")
    paths = set()
    for pattern in members:
        require(isinstance(pattern, str) and re.fullmatch(r"[A-Za-z0-9_.*?/-]+", pattern)
                and not pattern.startswith("/") and all(p not in ("", ".", "..") for p in pattern.split("/"))
                and "**" not in pattern, "unsupported Cargo member pattern")
        matches = list(source.glob(pattern))
        require(matches, "Cargo member pattern matches nothing")
        for path in matches:
            regular(path / "Cargo.toml")
            paths.add(path)
    if "package" in root:
        paths.add(source)
    packages = {}
    by_path = {}
    for path in sorted(paths):
        value = root if path == source else manifest(path / "Cargo.toml")
        require(path == source or "workspace" not in value, "nested Cargo workspace unsupported")
        package = value["package"]
        require("workspace" not in package, "explicit Cargo workspace override unsupported")
        name = package["name"]
        require(isinstance(name, str) and re.fullmatch(r"[a-z0-9_-]+", name) and name not in packages,
                "invalid or duplicate Cargo package")
        version = package["version"]
        if version == {"workspace": True}:
            version = workspace["package"]["version"]
        require(isinstance(version, str) and re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?", version),
                "unsupported Cargo version inheritance")
        publish = package.get("publish", True)
        if isinstance(publish, dict):
            require(publish == {"workspace": True}, "unsupported Cargo publish inheritance")
            publish = workspace["package"]["publish"]
        require(publish is True or publish is False or publish in ([], ["crates-io"]), "unsupported Cargo registry")
        packages[name] = (path, value, version, publish is not False and publish != [])
        by_path[path] = name
    dependencies = {}
    for name, (path, value, version, publish) in packages.items():
        local = set()
        tables = [value, *value.get("target", {}).values()]
        for table in tables:
            for kind in ("dependencies", "build-dependencies", "dev-dependencies"):
                for alias, dependency in table.get(kind, {}).items():
                    base = path
                    if isinstance(dependency, str):
                        continue
                    require(isinstance(dependency, dict), "unsupported Cargo dependency")
                    if "workspace" in dependency:
                        require(dependency["workspace"] is True
                                and set(dependency) <= {"workspace", "features", "optional", "default-features"},
                                "unsupported Cargo dependency inheritance")
                        dependency = workspace.get("dependencies", {})[alias]
                        base = source
                        if isinstance(dependency, str):
                            continue
                    if "path" not in dependency:
                        continue
                    relative = dependency["path"]
                    require(isinstance(relative, str) and not Path(relative).is_absolute(), "external Cargo dependency")
                    target = Path(os.path.abspath(base / relative))
                    require(target.is_relative_to(source) and target in by_path,
                            "implicit or external Cargo workspace member unsupported")
                    # Refuse path aliases through symlinks, including parents.
                    require((base / relative).resolve() == target, "symlink Cargo dependency")
                    target_name = by_path[target]
                    require(dependency.get("package", alias) == target_name, "Cargo dependency identity mismatch")
                    if publish and kind != "dev-dependencies":
                        require(packages[target_name][3], "unpublishable workspace dependency")
                        local.add(target_name)
        if publish:
            require(VERSION.fullmatch(version), "unsupported publishable Cargo version")
            dependencies[name] = sorted(local)
    require(dependencies, "no publishable Cargo workspace packages")
    result, emitted = [], set()
    while len(emitted) < len(dependencies):
        ready = sorted(name for name in dependencies if name not in emitted and set(dependencies[name]) <= emitted)
        require(ready, "cyclic package publication DAG")
        for name in ready:
            result.append({"registry": "crates.io", "name": name, "version": packages[name][2],
                           "dependencies": dependencies[name]})
            emitted.add(name)
    sdk = load_json(source / "typescript/package.json")
    require(sdk.get("name") == "@gripsack/core" and sdk.get("private", False) is False
            and isinstance(sdk.get("version"), str) and VERSION.fullmatch(sdk["version"]),
            "unsupported SDK publication identity")
    result.append({"registry": "npm", "name": sdk["name"], "version": sdk["version"], "dependencies": []})
    return result


def validate_inventory(source, packages):
    actual = [{key: package[key] for key in ("registry", "name", "version", "dependencies")} for package in packages]
    require(actual == source_inventory(source), "publication inventory/order differs from exact source workspace")
