#!/usr/bin/env python3
"""Linux-only, fail-closed replay/scheduled runner. Never executes targets unsandboxed."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from seeds import build

TARGETS = ("manifest", "journal", "merge", "store_gc", "archive")
ROOT = Path(__file__).resolve().parent


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("replay", "scheduled"))
    args = parser.parse_args()
    toolchain = os.environ.get("FUZZ_TOOLCHAIN")
    if not toolchain:
        raise SystemExit("FUZZ_TOOLCHAIN must be supplied by pinned infrastructure")
    bwrap = shutil.which("bwrap")
    if not bwrap:
        raise SystemExit("bubblewrap is required; no uncontained fallback")
    # Scheduled per-target libFuzzer budget; infrastructure may raise it
    # (e.g. weekly FUZZ_SECONDS=300). Replay mode is deterministic and
    # ignores this entirely.
    try:
        fuzz_seconds = int(os.environ.get("FUZZ_SECONDS", "60"))
    except ValueError:
        raise SystemExit("FUZZ_SECONDS must be an integer number of seconds")
    if not 1 <= fuzz_seconds <= 3600:
        raise SystemExit("FUZZ_SECONDS must be within 1..3600")
    compiler = subprocess.check_output(["rustc", "+" + toolchain, "-vV"], text=True, timeout=30)
    host_triple = next((line.removeprefix("host: ") for line in compiler.splitlines() if line.startswith("host: ")), None)
    if not host_triple or "/" in host_triple:
        raise SystemExit("pinned compiler did not report a valid host target")
    print(f"fuzz compiler: {compiler.splitlines()[0]} ({host_triple})", flush=True)
    with tempfile.TemporaryDirectory(prefix="gripsack-fuzz-build-") as staging:
        staging = Path(staging)
        target_dir = staging / "target"
        env = dict(os.environ, CARGO_TARGET_DIR=str(target_dir))
        # An explicit target separates build scripts/proc macros from target
        # instrumentation. Host executables do not link the libFuzzer runtime.
        cmd = ["cargo", "+" + toolchain, "build", "--locked", "--manifest-path", str(ROOT / "Cargo.toml"), "--release", "--target", host_triple]
        if args.mode == "scheduled":
            # inline-8bit-counters + pc-table drive coverage-guided evolution.
            # trace-compares is deliberately omitted: it arms libFuzzer's TORC
            # compare-hint machinery (Mutate_AddWordFromTORC → memmem), which
            # mislinks in the static musl C++ runtime and jumps through a null
            # slot — reproduced deterministically at seed 42; see plan 0042 E.
            env["RUSTFLAGS"] = "-Cdebug-assertions=yes -Coverflow-checks=yes -Cdebuginfo=1 -Cpasses=sancov-module -Cllvm-args=-sanitizer-coverage-level=4 -Cllvm-args=-sanitizer-coverage-inline-8bit-counters -Cllvm-args=-sanitizer-coverage-pc-table"
            cmd += ["--features", "fuzzing"]
            for target in TARGETS:
                cmd += ["--bin", target]
        else:
            env["RUSTFLAGS"] = "-Cdebug-assertions=yes -Coverflow-checks=yes"
            cmd += ["--bin", "replay"]
        subprocess.run(cmd, env=env, check=True, timeout=1800)
        for target in TARGETS:
            with tempfile.TemporaryDirectory(prefix="gripsack-fuzz-run-") as writable:
                work = Path(writable)
                shutil.copytree(ROOT / "corpus", work / "corpus")
                build(work / "corpus")
                (work / "artifacts").mkdir()
                # The staging tree (including the built binary) lives under
                # the host /tmp, which the sandbox replaces with a tmpfs —
                # expose it read-only at a private mount point instead.
                contained = [
                    bwrap, "--die-with-parent", "--new-session", "--unshare-all",
                    "--cap-drop", "ALL", "--ro-bind", "/", "/",
                    "--tmpfs", "/tmp", "--tmpfs", "/home", "--tmpfs", "/root",
                    "--ro-bind", str(staging), "/tmp/build",
                    "--proc", "/proc", "--dev", "/dev",
                    "--bind", str(work), "/tmp/work", "--chdir", "/tmp/work",
                    "--clearenv", "--setenv", "HOME", "/tmp",
                    "--setenv", "TMPDIR", "/tmp",
                    "--setenv", "GRIPSACK_HOME", "/tmp",
                    "--setenv", "GRIPSACK_FUZZ_ISOLATED", "1",
                    f"/tmp/build/target/{host_triple}/release/" + ("replay" if args.mode == "replay" else target),
                ]
                if args.mode == "replay":
                    contained += [target, f"/tmp/work/corpus/{target}"]
                else:
                    # Per-input budget: bounded archive decodes legitimately
                    # take seconds (bomb rejection streams to the expanded cap
                    # before failing); a 5s cut turned the timeout report
                    # itself into a crash before the reporting hooks existed.
                    contained += [f"/tmp/work/corpus/{target}", "-seed=42", "-max_len=65536",
                                  f"-max_total_time={fuzz_seconds}", "-timeout=25",
                                  "-rss_limit_mb=512",
                                  "-malloc_limit_mb=128", "-artifact_prefix=/tmp/work/artifacts/"]
                # OS limits apply to deterministic replay as well as libFuzzer.
                # The CPU cap scales with the libFuzzer budget so a longer
                # scheduled run is bounded by its own deadline, not cut at 90s.
                cpu_cap = 90 if args.mode == "replay" else fuzz_seconds + 30

                def limits():
                    import resource
                    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
                    resource.setrlimit(resource.RLIMIT_AS, (2 * 1024**3, 2 * 1024**3))
                    resource.setrlimit(resource.RLIMIT_FSIZE, (64 * 1024**2, 64 * 1024**2))
                    resource.setrlimit(resource.RLIMIT_CPU, (cpu_cap, cpu_cap))
                wall = max(100, fuzz_seconds + 40)
                subprocess.run(contained, check=True, timeout=wall, preexec_fn=limits)


if __name__ == "__main__":
    main()
