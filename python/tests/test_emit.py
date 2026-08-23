"""Frontend contract tests (plan/0003 §5): emit shape + provenance."""

import json

import pytest

import gripsack
from gripsack import (
    Module,
    clear_graph,
    clear_resources,
    config_step,
    dep,
    emit_ir,
    fetch_step,
    github_release,
    install_step,
    module,
    resource,
    service,
    shell_step,
    symlink,
    tarball,
    tracked_copy,
)


def setup_function():
    clear_graph()
    clear_resources()


def test_emit_shape_matches_ir_v1():
    module(
        "helix",
        fetch=github_release(
            repo="helix-editor/helix",
            asset="helix-{version}-x86_64-linux.tar.xz",
        ),
        install={"bin/hx": symlink("~/.local/bin/hx")},
        config={"config.toml": tracked_copy("~/.config/helix/config.toml")},
        depends=[dep("git")],
        activate=[service("syncthing")],
    )
    module("git", fetch=tarball("https://example.invalid/git.tar.xz"))

    ir = json.loads(emit_ir(tags=["gui"]))

    assert ir["ir_version"] == 1
    assert ir["host"]["tags"] == ["gui"]
    assert ir["host"]["os"] and ir["host"]["arch"]

    helix = ir["modules"]["helix"]
    assert helix["fetch"]["kind"] == "github_release"
    assert helix["fetch"]["repo"] == "helix-editor/helix"
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
    module("x", fetch=tarball("https://example.invalid/x.tar.xz"))
    ir = json.loads(emit_ir())
    span = ir["modules"]["x"]["span"]
    assert span["file"].endswith("test_emit.py")
    assert isinstance(span["line"], int) and span["line"] > 0


def test_build_edge_is_declared():
    module(
        "helix-src",
        fetch=tarball("https://example.invalid/helix.tar.xz"),
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
    assert "fetch" not in helix
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


def test_declared_resources_emit_and_validate():
    resource("company.lock")
    module(
        "tool",
        steps=[
            shell_step("./sync.sh", id="sync", resources=["company.lock"]),
        ],
    )
    ir = json.loads(emit_ir())
    assert ir["resources"] == [{"name": "company.lock"}]
    assert ir["modules"]["tool"]["steps"][0]["resources"] == ["company.lock"]


def test_undeclared_resource_raises_at_eval():
    with pytest.raises(ValueError, match="unknown resource 'cargo.lokc'"):
        shell_step("true", id="x", resources=["cargo.lokc"])


def test_builtin_resources_pass_eval():
    s = shell_step("cargo install hx", id="build", resources=["cargo-lock"])
    assert s.resources == ["cargo-lock"]


def test_class_module_chains_pipeline():
    class Helix(Module):
        def fetch(self):
            return fetch_step(tarball("https://example.invalid/h.tar.xz"))

        def install(self):
            return install_step({"bin/hx": symlink("~/.local/bin/hx")})

        def config(self):
            return config_step(
                {"config.toml": tracked_copy("~/.config/helix/config.toml")}
            )

    ir = json.loads(emit_ir())
    steps = ir["modules"]["helix"]["steps"]
    assert [s["id"] for s in steps] == ["fetch", "install", "config"]
    # implicit sequencing compiled to explicit needs
    assert "needs" not in steps[0]
    assert steps[1]["needs"] == ["fetch"]
    assert steps[2]["needs"] == ["install"]
    # phase tags filled from the pipeline
    assert steps[2]["phase"] == "config"
    # span points at the class definition
    assert ir["modules"]["helix"]["span"]["file"].endswith("test_emit.py")


def test_class_module_explicit_needs_win_and_abstract_bases_skip():
    class Base(Module):
        abstract = True

        def fetch(self):
            return fetch_step(tarball("https://example.invalid/x.tar.xz"))

    class Tool(Base):
        def build(self):
            return shell_step("make", id="make", needs=["fetch"])

    ir = json.loads(emit_ir())
    assert "base" not in ir["modules"]
    steps = ir["modules"]["tool"]["steps"]
    # explicit needs are preserved, not chained
    assert steps[1]["needs"] == ["fetch"]


def test_duplicate_module_names_error_at_eval():
    module("dup", fetch=tarball("https://example.invalid/a.tar.xz"))
    with pytest.raises(ValueError, match="duplicate module 'dup'"):
        module("dup", fetch=tarball("https://example.invalid/b.tar.xz"))


def test_when_filters_data_style_modules():
    from gripsack import when as mkwhen

    module(
        "steam",
        fetch=tarball("https://example.invalid/s.tar.xz"),
        when=mkwhen(os="plan9"),
    )
    module("git", fetch=tarball("https://example.invalid/g.tar.xz"))
    ir = json.loads(emit_ir())
    assert "steam" not in ir["modules"]
    assert "git" in ir["modules"]


def test_when_decorator_filters_class_modules():
    from gripsack import when as mkwhen

    @mkwhen(os="plan9")
    class Steam(Module):
        def fetch(self):
            return fetch_step(tarball("https://example.invalid/s.tar.xz"))

    ir = json.loads(emit_ir())
    assert "steam" not in ir["modules"]


def test_facts_object_has_tags_after_runner_sets():
    import sys

    from gripsack._facts import _set_tags

    _set_tags(["gui", "laptop"])
    assert sys.modules["gripsack._facts"].facts.has("gui")
    _set_tags([])


def test_version_string_exists():
    assert gripsack.__version__
