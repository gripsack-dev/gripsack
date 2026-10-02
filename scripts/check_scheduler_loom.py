#!/usr/bin/env python3
"""M-V2: run the actual coordinator with Loom and calibrate missed completions/wakes.

Two workers, four-node diamond or three-node failure/panic graph, at most two
preemptions. This is bounded systematic testing, not a universal safety proof.
No fuzz target or fuzz corpus is executed.
"""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
COORDINATOR = Path("crates/gripsack-exec/src/schedule/coordination.rs")
MODEL = "schedule::coordination::model::"
DIAMOND = MODEL + "diamond_completion_wakes_waiters_and_publishes_dependencies"
EXPECTED = (
    "diamond_completion_wakes_waiters_and_publishes_dependencies",
    "failure_latches_and_drains_without_lost_notification",
    "panic_completes_once_and_wakes_waiters",
)


def run(root: Path, target: Path, selection: str) -> subprocess.CompletedProcess:
    command = [
        "cargo", "test", "--locked", "-p", "gripsack-exec", "--features", "loom-tests",
        "--lib", selection,
    ]
    print("RUNNER_COMMAND=" + " ".join(command), flush=True)
    result = subprocess.run(
        command, cwd=root, env={**os.environ, "CARGO_TARGET_DIR": str(target)},
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=600,
    )
    print(result.stdout, flush=True)
    return result


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="gripsack-loom-") as temporary:
        temporary = Path(temporary)
        tree = temporary / "source"
        tree.mkdir()
        # Use the same production workspace/dependency graph; no shadow crate
        # or hand-copied coordination algorithm can make this gate pass.
        for name in ("Cargo.toml", "Cargo.lock"):
            shutil.copy2(ROOT / name, tree / name)
        for name in ("crates", "fuzz", "typescript", "schema"):
            shutil.copytree(ROOT / name, tree / name, ignore=shutil.ignore_patterns(
                "target", "node_modules", ".venv", "__pycache__",
            ))
        target = temporary / "target"
        positive = run(tree, target, MODEL)
        if positive.returncode or "test result: ok. 3 passed; 0 failed" not in positive.stdout:
            raise SystemExit("FAIL: production coordinator Loom cases did not all pass")
        for case in EXPECTED:
            if f"test {MODEL}{case} ... ok" not in positive.stdout:
                raise SystemExit(f"FAIL: named Loom case {case} did not execute")
        print("LOOM_SCHEDULER=3", flush=True)
        source = tree / COORDINATOR
        original = source.read_text()
        mutants = (
            ("missing-notification", "        self.changed.notify_all();"),
            ("missing-completion", "        state.running -= 1;"),
        )
        for name, statement in mutants:
            if original.count(statement) != 1:
                raise SystemExit(f"FAIL: {name} mutation no longer uniquely matches")
            source.write_text(original.replace(statement, ""))
            result = run(tree, target, DIAMOND)
            source.write_text(original)
            # Loom, not a process deadline, observes the stuck waiter in the
            # named real-coordinator scenario. Syntax/tool/unrelated panics
            # cannot calibrate the notification/completion boundary.
            if (result.returncode == 0 or "deadlock; threads =" not in result.stdout
                    or "could not compile" in result.stdout):
                raise SystemExit(f"FAIL: {name} did not trigger Loom's waiter deadlock")
            print(f"calibration: {name} rejected by Loom waiter deadlock", flush=True)
        print("LOOM_SCHEDULER_MUTANTS=2", flush=True)


if __name__ == "__main__":
    main()
