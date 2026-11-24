"""I/O failures name the path, the output and the declaration (#401, #408)."""
import json
import os

from conftest import grip, make_toolchain_tarball

HEAD = 'import {defineWorkspace, workspace, pkg, provider, tarball, profile, file, literalText, trackedCopyTo} from "@gripsack/core";\n'


def repo_with(sandbox, body):
    repo = sandbox / "repo"
    repo.mkdir()
    (repo / "gripsack.ts").write_text(HEAD + body)
    return repo


def package_repo(sandbox, member, command):
    archive = sandbox / "tool.tar.gz"
    make_toolchain_tarball(archive, {member: b"#!/bin/sh\necho hi\n"})
    return repo_with(sandbox, f'''export default defineWorkspace(() => workspace({{outputs:[
  pkg("xh", {{producer:provider(tarball({json.dumps(archive.as_uri())})),
    commands:{{xhs:{json.dumps(command)}}},target:{{os:"linux",arch:"x86_64"}},layout:{{kind:"relocatable"}}}}),
]}}));
''')


def test_missing_command_names_package_command_path_and_span(sandbox):
    repo = package_repo(sandbox, "xh-v1/xh", "xhs")
    assert grip("update", cwd=repo).returncode in (0, 1)
    result = grip("build", "xh", cwd=repo)
    assert result.returncode != 0
    err = result.stderr
    assert 'package "xh"' in err, err
    assert 'command "xhs"' in err and "not found" in err, err
    assert "xh-v1/" in err, err  # archive top level hint
    assert "gripsack.ts:" in err, err
    assert "io:" not in err, err


def test_dangling_parent_symlink_destination_is_a_named_conflict(sandbox):
    (sandbox / ".config").mkdir()
    (sandbox / ".config/lazygit").symlink_to(sandbox / "dotfiles-gone/lazygit")
    repo = repo_with(sandbox, '''export default defineWorkspace(() => workspace({outputs:[
  profile("dots", {files:[file({content:literalText("x\\n"),destination:trackedCopyTo("~/.config/lazygit/config.yml")})]}),
]}));
''')
    result = grip("apply", "dots", cwd=repo)
    assert result.returncode != 0
    err = result.stderr
    assert "lazygit" in err and "dangling" in err, err
    assert 'profile "dots"' in err, err
    assert "gripsack.ts:" in err, err
    assert "io:" not in err, err
