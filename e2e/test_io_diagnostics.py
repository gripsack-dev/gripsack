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


LIB = '''import {pkg, provider, tarball} from "@gripsack/core";
export function release(name: string, url: string) {
  return pkg(name, {producer: provider(tarball(url)),
    commands: {[name]: name}, target: {os: "linux", arch: "x86_64"}, layout: {kind: "relocatable"}});
}
'''


def helper_repo(sandbox, names, url):
    repo = sandbox / "helpers"
    repo.mkdir()
    (repo / "lib.ts").write_text(LIB)
    calls = "\n".join(f'  release("{n}", {json.dumps(url)}),' for n in names)
    (repo / "gripsack.ts").write_text(
        'import {defineWorkspace, workspace} from "@gripsack/core";\n'
        'import {release} from "./lib.ts";\n'
        f"export default defineWorkspace(() => workspace({{outputs:[\n{calls}\n]}}));\n"
    )
    return repo


def test_helper_diagnostics_point_at_the_caller_and_name_the_output(sandbox):
    archive = sandbox / "tool.tar.gz"
    make_toolchain_tarball(archive, {"tool": b"#!/bin/sh\n"})
    repo = helper_repo(sandbox, ["hunk", "delta"], archive.as_uri())
    # Frozen acquisition without a lock: the failure is per output.
    result = grip("build", "delta", cwd=repo)
    assert result.returncode != 0
    err = result.stderr
    assert 'package "delta"' in err, err
    assert "gripsack.ts:5:" in err, err  # the release("delta", ...) call
    assert "lib.ts" not in err, err


def test_duplicate_helper_outputs_label_each_call_site_in_json(sandbox):
    repo = helper_repo(sandbox, ["hunk", "hunk"], "file:///nonexistent.tar.gz")
    machine = grip("check", "--json", cwd=repo)
    assert machine.returncode != 0
    doc = json.loads(machine.stdout)
    labels = [label for d in doc["diagnostics"] for label in d["labels"]]
    assert labels, doc
    spans = {(label["span"]["file"].rsplit("/", 1)[-1], label["span"]["line"]) for label in labels}
    assert spans == {("gripsack.ts", 4), ("gripsack.ts", 5)}, spans
