"""Build closures (0039): real offline artifacts, real frontend, real history.
The boundary is observable: tools run from the store, never deploy by accident,
and retained generations keep the exact build inputs alive.
"""

import json
import shlex
from pathlib import Path

import pytest

from conftest import grip, make_env_repo, make_toolchain_tarball, refresh_host, remove_module


def run(repo, *args):
    out = grip(*args, cwd=repo)
    assert out.returncode == 0, out.stdout + out.stderr
    return out


def apply(repo, *modules):
    return run(repo, "apply", *modules, "--host", "testhost")


def manifest(sandbox):
    current = sandbox / ".local/share/gripsack/current"
    return json.loads((current.resolve() / "manifest.json").read_text())


def lines(path):
    return path.read_text().splitlines() if path.exists() else []


def compiler_source(payload, sandbox, depends="", verify=None):
    check = verify or f"echo checked >> {shlex.quote(str(sandbox / 'checks'))}"
    hook = f"echo activated >> {shlex.quote(str(sandbox / 'hooks'))}"
    return f'''
import {{ customHook, dep, fileFetch, module, symlink, verifyShell }} from "@gripsack/core";
export default module("compiler", {{
  fetch: fileFetch({json.dumps(str(payload))}),
  depends: [{depends}],
  install: {{ "bin/cc": symlink("~/.local/bin/cc") }},
  verify: verifyShell({json.dumps(check)}),
  env: {{ COMPILER_PROFILE: "must-not-leak" }},
  activate: [customHook({json.dumps(hook)})],
}});
'''


def consumer_source(sandbox, depends='dep("compiler", { for: "build" })', script=None):
    build_log = shlex.quote(str(sandbox / "builds"))
    script = script or (
        f"mkdir -p out && echo built >> {build_log} && "
        'cp "$(command -v cc)" out/built && printf %s "$GRIP_DEP_COMPILER" > out/depref'
    )
    return f'''
import {{ dep, installStep, module, shellStep, symlink }} from "@gripsack/core";
export default module("consumer", {{
  depends: [{depends}],
  steps: [
    shellStep({json.dumps(script)}, "build"),
    installStep({{ "out/built": symlink("~/.local/bin/built") }}, "install", {{ needs: ["build"] }}),
  ],
}});
'''


def fixture(sandbox, verify=None):
    payload = sandbox / "compiler.tar.gz"
    make_toolchain_tarball(payload, {"bin/cc": b"#!/bin/sh\necho compiler-v1\n"})
    repo = make_env_repo(sandbox / "env", {
        "compiler": compiler_source(payload, sandbox, verify=verify),
        "consumer": consumer_source(sandbox),
    })
    return repo, payload


def test_cold_warm_update_rollback_and_collection(sandbox):
    repo, payload = fixture(sandbox)
    built = sandbox / ".local/bin/built"
    compiler_bin = sandbox / ".local/bin/cc"
    preview = run(repo, "plan", "--host", "testhost")
    assert "fetch + stage for build" in preview.stdout
    assert "~/.local/bin/cc" not in preview.stdout
    apply(repo)
    first = manifest(sandbox)
    tool = first["modules"]["compiler"]
    root = tool["store_path"]
    assert tool["build_only"] and not tool["entries"]
    assert tool["verified"] and not tool.get("intents") and not tool.get("env")
    assert first["modules"]["consumer"]["build_closure"] == [root]
    assert built.read_bytes() == b"#!/bin/sh\necho compiler-v1\n"
    assert (built.resolve().parent / "depref").read_text() == root
    assert not compiler_bin.is_symlink() and not compiler_bin.exists()
    assert not (sandbox / "hooks").exists()

    # A TOFU pin learned earlier in this apply is already in the consumer key.
    # Warm apply must reuse BOTH the built artifact and payload verify receipt.
    apply(repo)
    assert manifest(sandbox)["number"] == first["number"]
    assert lines(sandbox / "builds") == ["built"]
    assert lines(sandbox / "checks") == ["checked"]

    make_toolchain_tarball(payload, {"bin/cc": b"#!/bin/sh\necho compiler-v2\n"})
    run(repo, "update", "--host", "testhost")
    apply(repo)
    second = manifest(sandbox)
    assert built.read_bytes() == b"#!/bin/sh\necho compiler-v2\n"
    assert len(lines(sandbox / "builds")) == 2
    assert len(lines(sandbox / "checks")) == 2
    apply(repo)
    assert manifest(sandbox)["number"] == second["number"]
    assert len(lines(sandbox / "builds")) == 2

    run(repo, "rollback", str(first["number"]))
    assert built.read_bytes() == b"#!/bin/sh\necho compiler-v1\n"
    assert len(lines(sandbox / "builds")) == 2, "rollback must never rebuild"
    assert len(lines(sandbox / "checks")) == 2
    assert not (sandbox / "hooks").exists()
    run(repo, "rollback", str(second["number"]))

    remove_module(repo, "consumer")
    remove_module(repo, "compiler")
    apply(repo)
    assert not built.is_symlink() and not compiler_bin.is_symlink()
    run(repo, "gc")
    assert Path(root).exists(), "retained history still pins the old closure"
    config = sandbox / ".config/gripsack"
    config.mkdir(parents=True, exist_ok=True)
    (config / "config.toml").write_text("[settings]\nkeep_generations = 1\n")
    run(repo, "gc")
    assert not Path(root).exists()
    assert not list((sandbox / ".local/share/gripsack/store").iterdir())


def test_build_only_verify_failure_is_not_a_receipt(sandbox):
    gate = shlex.quote(str(sandbox / "allow-check"))
    counter = shlex.quote(str(sandbox / "checks"))
    repo, payload = fixture(sandbox, verify=f"echo checked >> {counter}; test -f {gate}")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0 and "E302" in out.stderr
    assert not (sandbox / ".local/share/gripsack/current").exists()
    assert not (sandbox / "builds").exists()
    (sandbox / "allow-check").touch()
    apply(repo)
    assert len(lines(sandbox / "checks")) == 2, "store presence is not a receipt"
    apply(repo)
    assert len(lines(sandbox / "checks")) == 2
    # Empty destination lists cannot make a receipt valid for different bytes.
    (sandbox / "allow-check").unlink()
    make_toolchain_tarball(payload, {"bin/cc": b"#!/bin/sh\necho changed\n"})
    run(repo, "update", "--host", "testhost")
    out = grip("apply", "--host", "testhost", cwd=repo)
    assert out.returncode != 0 and "E302" in out.stderr
    assert len(lines(sandbox / "checks")) == 3
    assert (sandbox / ".local/bin/built").read_bytes().endswith(b"compiler-v1\n")


def test_transitive_diamond_path_and_runtime_boundary(sandbox):
    repo, payload = fixture(sandbox)
    sdk = sandbox / "sdk.tar.gz"
    runtime = sandbox / "runtime.tar.gz"
    make_toolchain_tarball(sdk, {"bin/cc": b"#!/bin/sh\necho sdk-v1\n"})
    make_toolchain_tarball(runtime, {"bin/runtime": b"#!/bin/sh\necho runtime\n"})
    (repo / "modules/sdk.ts").write_text(f'''
import {{ fileFetch, module, symlink }} from "@gripsack/core";
export default module("sdk", {{ fetch: fileFetch({json.dumps(str(sdk))}),
  install: {{ "bin/cc": symlink("~/.local/bin/cc") }} }});
''')
    (repo / "modules/runtime.ts").write_text(f'''
import {{ fileFetch, module, symlink }} from "@gripsack/core";
export default module("runtime", {{ fetch: fileFetch({json.dumps(str(runtime))}),
  install: {{ "bin/runtime": symlink("~/.local/bin/runtime") }} }});
''')
    (repo / "modules/compiler.ts").write_text(compiler_source(
        payload, sandbox, depends='dep("sdk", { for: "build" }), dep("runtime")',
    ))
    # A structured run step overrides PATH, but the closure must still prepend.
    (repo / "modules/consumer.ts").write_text('''
import { dep, installStep, module, runStep, symlink } from "@gripsack/core";
export default module("consumer", {
  depends: [dep("compiler", { for: "build" }), dep("sdk", { for: "build" })],
  steps: [
    runStep(["sh", "-c", "mkdir -p out; cp $(command -v cc) out/built; printf %s $PATH > out/path"], "build", {
      env: { PATH: "/usr/bin:/bin" }, outputs: ["out/built"],
    }),
    installStep({ "out/built": symlink("~/.local/bin/built") }, "install", { needs: ["build"] }),
  ],
});
''')
    refresh_host(repo)
    apply(repo)
    states = manifest(sandbox)["modules"]
    closure = states["consumer"]["build_closure"]
    assert closure == [states["sdk"]["store_path"], states["compiler"]["store_path"]]
    built = sandbox / ".local/bin/built"
    assert built.read_bytes().endswith(b"sdk-v1\n"), "dependency-first PATH chooses sdk before compiler"
    path = (built.resolve().parent / "path").read_text()
    assert path == ":".join([p + "/bin" for p in closure] + ["/usr/bin", "/bin"])
    assert (sandbox / ".local/bin/runtime").is_symlink()
    assert not (sandbox / ".local/bin/cc").is_symlink(), "colliding build-only destinations do not deploy"
    make_toolchain_tarball(sdk, {"bin/cc": b"#!/bin/sh\necho sdk-v2\n"})
    run(repo, "update", "--host", "testhost")
    apply(repo)
    assert built.read_bytes().endswith(b"sdk-v2\n"), "transitive pin updates must rebuild the consumer"


def test_runtime_role_transitions_and_subset_retention(sandbox):
    repo, _ = fixture(sandbox)
    (repo / "payload").write_text("untouched\n")
    (repo / "modules/unrelated.ts").write_text('''
import { module, trackedCopy } from "@gripsack/core";
export default module("unrelated", { config: { payload: trackedCopy("~/.unrelated") } });
''')
    # Runtime incoming edges win even when another consumer only needs a build tool.
    user = repo / "modules/user.ts"
    user.write_text('''
import { dep, module } from "@gripsack/core";
export default module("user", { depends: [dep("compiler")] });
''')
    refresh_host(repo)
    apply(repo)
    compiler_bin = sandbox / ".local/bin/cc"
    assert compiler_bin.is_symlink()
    assert lines(sandbox / "hooks") == ["activated"]
    home = sandbox / ".local/share/gripsack"
    assert "COMPILER_PROFILE" in (home / "current/env/profile.sh").read_text()
    unrelated = manifest(sandbox)["modules"]["unrelated"]

    user.write_text('''
import { dep, module } from "@gripsack/core";
export default module("user", { depends: [dep("compiler", { for: "build" })] });
''')
    apply(repo, "consumer", "user")
    assert manifest(sandbox)["modules"]["unrelated"] == unrelated
    assert (sandbox / ".unrelated").read_text() == "untouched\n"
    assert not compiler_bin.is_symlink(), "runtime-to-build prunes the old deployment"
    profile = home / "current/env/profile.sh"
    assert not profile.exists() or "COMPILER_PROFILE" not in profile.read_text()
    assert lines(sandbox / "hooks") == ["activated"]
    user.write_text('''
import { dep, module } from "@gripsack/core";
export default module("user", { depends: [dep("compiler")] });
''')
    apply(repo, "user")
    assert compiler_bin.is_symlink()
    assert lines(sandbox / "hooks") == ["activated", "activated"]


@pytest.mark.parametrize("purpose", ['"buidl"', "null"])
def test_check_rejects_unknown_purpose_with_source_label(sandbox, purpose):
    repo = make_env_repo(sandbox / "env", {
        "tool": 'import { module } from "@gripsack/core"; export default module("tool", {});',
        "consumer": f'''import {{ dep, module }} from "@gripsack/core";
export default module("consumer", {{ depends: [dep("tool", {{ for: {purpose} as never }})] }});
''',
    })
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0
    assert "E122" in out.stderr and "consumer.ts:2:" in out.stderr


def test_ambiguous_build_dependency_export_is_rejected(sandbox):
    repo = make_env_repo(sandbox / "env", {
        "a": 'import { module } from "@gripsack/core"; export default module("a-b", {});',
        "b": 'import { module } from "@gripsack/core"; export default module("a_b", {});',
        "consumer": '''import { dep, module } from "@gripsack/core";
export default module("consumer", { depends: [dep("a-b", { for: "build" }), dep("a_b", { for: "build" })] });
''',
    })
    out = grip("check", "--host", "testhost", cwd=repo)
    assert out.returncode != 0 and "E123" in out.stderr
    assert "consumer.ts:2:" in out.stderr
