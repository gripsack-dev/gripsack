#!/usr/bin/env python3
"""M-V5 deterministic production GC histories and dropped-root calibration.

No random input, fuzz target or corpus replay. Only the named filesystem oracle
can calibrate the production root omission; compiler errors/timeouts fail.
"""
from pathlib import Path
import os
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
COLLECTOR = Path("crates/gripsack-exec/src/gc.rs")
PREFIX = "gc::root_model::"
HISTORY = PREFIX + "retained_histories_protect_complete_roots"
EXPECTED = (
    "retained_histories_protect_complete_roots",
    "unfinished_recovery_preserves_every_history_object",
    "pinned_collection_survives_root_replacement",
)
MINIMUMS = {
    "GC_RETAINED_HISTORIES": 7,
    "GC_RECOVERY_ADMISSIONS": 8,
    "GC_PINNED_ROOT_REPLACEMENTS": 1,
}


def run(root, target, selection, exact=False):
    command = ["cargo", "test", "--locked", "-p", "gripsack-exec", "--lib",
               selection, "--", "--nocapture"]
    if exact:
        command.append("--exact")
    print("RUNNER_COMMAND=" + " ".join(command), flush=True)
    result = subprocess.run(command, cwd=root,
                            env={**os.environ, "CARGO_TARGET_DIR": str(target)},
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            text=True, timeout=1200)
    print(result.stdout, flush=True)
    return result


def main():
    with tempfile.TemporaryDirectory(prefix="gripsack-gc-roots-") as temporary:
        temporary = Path(temporary)
        tree = temporary / "source"
        tree.mkdir()
        for name in ("Cargo.toml", "Cargo.lock"):
            shutil.copy2(ROOT / name, tree / name)
        for name in ("crates", "fuzz", "typescript", "schema"):
            shutil.copytree(ROOT / name, tree / name, ignore=shutil.ignore_patterns(
                "target", "node_modules", ".venv", "__pycache__",
            ))
        target = temporary / "target"
        positive = run(tree, target, PREFIX)
        if positive.returncode or "test result: ok. 3 passed; 0 failed; 0 ignored" not in positive.stdout:
            raise SystemExit("FAIL: complete production GC history properties did not pass")
        for name in EXPECTED:
            if f"test {PREFIX}{name} ... ok" not in positive.stdout:
                raise SystemExit(f"FAIL: named GC property did not run: {name}")
        counts = dict((name, int(count)) for name, count in re.findall(
            r"\b(GC_[A-Z_]+)=(\d+)\b", positive.stdout))
        for name, minimum in MINIMUMS.items():
            if counts.get(name, 0) < minimum:
                raise SystemExit(f"FAIL: missing or narrowed history family {name}")
        print("GC_ROOT_PROPERTIES=3", flush=True)
        source = tree / COLLECTOR
        original = source.read_text()
        root = "referenced.insert(utf8_path(path)?);"
        if original.count(root) != 1:
            raise SystemExit("FAIL: build-closure root mutation no longer uniquely matches")
        source.write_text(original.replace(root, "let _ = path;"))
        try:
            negative = run(tree, target, HISTORY, exact=True)
        finally:
            source.write_text(original)
        if (negative.returncode == 0
                or f"test {HISTORY} ... FAILED" not in negative.stdout
                or "retained_history_root_missing: old-compiler" not in negative.stdout
                or "test result: FAILED. 0 passed; 1 failed; 0 ignored" not in negative.stdout
                or "could not compile" in negative.stdout):
            raise SystemExit("FAIL: dropped build root did not fail the named production history oracle")
        print("calibration: dropped-build-closure-root rejected by real retained history", flush=True)
        print("GC_ROOT_MUTANTS=1", flush=True)
        print("GC root gate: OK", flush=True)


if __name__ == "__main__":
    try:
        main()
    except subprocess.TimeoutExpired as error:
        raise SystemExit("FAIL: GC timeout is not semantic calibration") from error
