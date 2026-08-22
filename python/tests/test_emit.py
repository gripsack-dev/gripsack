"""Frontend contract tests (plan/0003 §5): emit shape + provenance."""

import json

import gripsack
from gripsack import (
    clear_graph,
    dep,
    emit_ir,
    fetch_step,
    github_release,
    module,
    service,
    shell_step,
    symlink,
    tarball,
    tracked_copy,
)


def setup_function():
    clear_graph()


def test_emit_shape_matches_ir_v1():
    module(
        "helix",
        source=github_release(
            repo="helix-editor/helix",
            asset="helix-{version}-x86_64-linux.tar.xz",
        ),
        install={"bin/hx": symlink("~/.local/bin/hx")},
        config={"config.toml": tracked_copy("~/.config/helix/config.toml")},
        depends=[dep("git")],
        activate=[service("syncthing")],
    )
    module("git", source=tarball("https://example.invalid/git.tar.xz"))

    ir = json.loads(emit_ir(tags=["gui"]))

    assert ir["ir_version"] == 1
    assert ir["host"]["tags"] == ["gui"]
    assert ir["host"]["os"] and ir["host"]["arch"]

    helix = ir["modules"]["helix"]
    assert helix["source"]["kind"] == "github_release"
    assert helix["source"]["repo"] == "helix-editor/helix"
    assert helix["install"] == [
        {"from": "bin/hx", "to": "~/.local/bin/hx", "mode": "owned"}
    ]
    assert helix["config"][0]["mode"] == "tracked_copy"
    assert helix["depends"] == [{"module": "git", "edge": "runtime"}]
    assert helix["activate"][0]["kind"] == "service"
    assert helix["activate"][0]["trigger"] == "post_activate"

    # optional sections absent, not null — the IR is sparse by convention
    git_mod = ir["modules"]["git"]
    assert "install" not in git_mod
    assert "depends" not in git_mod


def test_span_points_at_this_file():
    module("x", source=tarball("https://example.invalid/x.tar.xz"))
    ir = json.loads(emit_ir())
    span = ir["modules"]["x"]["span"]
    assert span["file"].endswith("test_emit.py")
    assert isinstance(span["line"], int) and span["line"] > 0


def test_build_edge_is_declared():
    module(
        "helix-src",
        source=tarball("https://example.invalid/helix.tar.xz"),
        depends=[dep("rust", edge="build")],
    )
    ir = json.loads(emit_ir())
    assert ir["modules"]["helix-src"]["depends"][0]["edge"] == "build"


def test_dotfiles_only_module_emits_no_source():
    module(
        "helix",
        config={"config.toml": tracked_copy("~/.config/helix/config.toml")},
    )
    ir = json.loads(emit_ir())
    helix = ir["modules"]["helix"]
    assert "source" not in helix
    assert helix["config"][0]["mode"] == "tracked_copy"


def test_explicit_steps_emit():
    module(
        "helix-patched",
        steps=[
            fetch_step(tarball("https://example.invalid/helix.tar.xz")),
            shell_step("patch -p1 < fix.patch", id="patch", needs=["fetch"]),
        ],
    )
    ir = json.loads(emit_ir())
    steps = ir["modules"]["helix-patched"]["steps"]
    assert steps[0]["action"]["kind"] == "fetch"
    assert steps[0]["phase"] == "fetch"
    assert steps[1] == {
        "id": "patch",
        "action": {"kind": "custom_shell", "script": "patch -p1 < fix.patch"},
        "needs": ["fetch"],
        "phase": "custom",
    }


def test_version_string_exists():
    assert gripsack.__version__
