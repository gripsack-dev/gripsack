"""Typed command selectors use each referenced lock, never literal interpolation."""
import io
import json
import tarfile

import pytest

from conftest import grip, make_toolchain_tarball, make_workspace_repo
from test_workspace_release_paths import PLATFORM, TOOL_TEMPLATE, evidence, releases, run


def publish_rooted(server, revision):
    """Reuse the release server; wrap only its tree asset for archiveRoot coverage."""
    server.publish(revision)
    name = server.names["tree", server.tags["tree"]]
    wrapped = io.BytesIO()
    with tarfile.open(fileobj=io.BytesIO(server.assets[name]), mode="r:gz") as source:
        with tarfile.open(fileobj=wrapped, mode="w:gz") as destination:
            for member in source:
                data = source.extractfile(member)
                member.name = "bundle/" + member.name
                destination.addfile(member, data)
    server.assets[name] = wrapped.getvalue()


def artifact_workspace(sandbox, server):
    shell = make_toolchain_tarball(sandbox / "shell.tar.gz", {
        "bin/bash": b'#!/bin/bash\nexec /bin/bash "$@"\n',
    })
    literal_cwd = sandbox / "literal-{version}-{os}"
    literal_cwd.mkdir()
    source = r'''import {
  pkg, provider, githubRelease, tarball, environment, task, check, hook, profile,
  exec, runBash, lit, artifact, sourcePath, packageCommand,
} from "@gripsack/core";
const target = {os:"linux", arch:"x86_64"} as const;
const layout = {kind:"relocatable"} as const;
const source = (repo: string, asset: string) => provider(githubRelease({
  repo:"acme/" + repo, asset, base_url:BASE,
}));
export default [
  pkg("tool", {producer:source("tool", TOOL), commands:{}, target, layout}),
  pkg("library", {producer:source("library", "libflyline.so.{version.bare}"), commands:{}, target, layout}),
  pkg("tree", {producer:source("tree", "tree-{version}.tar.gz"), commands:{}, target, layout,
    archiveRoot:{kind:"single_directory"}}),
  pkg("shell", {producer:provider(tarball(SHELL)), commands:{bash:"bin/bash"}, target, layout}),
  environment("tools", {packages:["shell"], target, env:{
    LIBRARY:artifact("library", "libflyline.so.{version.bare}"), LITERAL:lit("{version}/{os}"),
  }}),
  check("reported", {subject:"library", run:exec({argv:[
    lit("/usr/bin/test"), lit("-f"), artifact("library", "libflyline.so.{version.bare}"),
  ]})}),
  check("subject", {subject:"library", run:exec({argv:[
    lit("/bin/cat"), sourcePath("libflyline.so.{version.bare}"),
  ], env:{SUBJECT:sourcePath("libflyline.so.{version.bare}")}})}),
  check("subject-cwd", {subject:"tree", run:exec({
    argv:[lit("/bin/sh"), lit("-c"), lit('/bin/cat value "$SUBJECT"')],
    cwd:sourcePath("share/{version}"), env:{SUBJECT:sourcePath("share/{version}/value")},
  })}),
  task("probe", {environment:"tools", checks:["reported", "subject", "subject-cwd"], steps:[
    exec({argv:[artifact("tool", TOOL)]}),
    exec({argv:[lit("/bin/sh"), lit("-c"), lit('/bin/cat "$LIBRARY" "$COMMAND_LIBRARY" value; printf "literal:%s:%s\\n" "$LITERAL" "$1"'), lit("probe"), lit("{version}/{os}")],
      env:{COMMAND_LIBRARY:artifact("library", "libflyline.so.{version.bare}")},
      cwd:artifact("tree", "share/{version}")}),
    exec({argv:[lit("/bin/sh"), lit("-c"), lit('test -d "$1/share" && printf "whole-artifact\\n"'), lit("probe"), artifact("tree", ".")]}),
    exec({argv:[lit("/bin/sh"), lit("-c"), lit('printf "literal-cwd:%s\\n" "$PWD"')], cwd:lit(LITERAL_CWD)}),
    runBash({interpreter:packageCommand("shell", "bash"),
      body:'/bin/cat "$BASH_LIBRARY" value; printf "body:{version}/{os}\\n"',
      env:{BASH_LIBRARY:artifact("library", "libflyline.so.{version.bare}")},
      cwd:artifact("tree", "share/{version}")}),
  ]}),
  hook("retain", {trigger:"post_link", run:exec({
    argv:[lit("/bin/sh"), lit("-c"), lit('/bin/cat "$LIBRARY" value >> "$EFFECT"')],
    env:{LIBRARY:artifact("library", "libflyline.so.{version.bare}"), EFFECT:lit(EFFECT)},
    cwd:artifact("tree", "share/{version}"),
  })}),
  profile("retained", {hooks:["retain"]}),
];'''
    for token, value in (("BASE", server.base), ("TOOL", TOOL_TEMPLATE),
                         ("SHELL", shell.as_uri()), ("LITERAL_CWD", str(literal_cwd)),
                         ("EFFECT", str(sandbox / "hook-effects"))):
        # Replace placeholder values, not same-named environment object keys.
        if token == "EFFECT":
            source = source.replace("lit(EFFECT)", "lit(" + json.dumps(value) + ")")
        else:
            source = source.replace(token, json.dumps(value))
    return make_workspace_repo(sandbox / "artifact-paths", {"selectors": source})


def assert_artifact_plan(repo, server, revision):
    before = list(server.requests)
    result = run(repo, "plan", "--json")
    rows = [row for row in json.loads(result.stdout)["resolved_paths"] if row["kind"] == "artifact"]
    expected = {
        ("tools", "library", f"libflyline.so.{revision + 10}"),
        ("reported", "library", f"libflyline.so.{revision + 10}"),
        ("subject", "library", f"libflyline.so.{revision + 10}"),
        ("subject-cwd", "tree", f"share/v{revision + 20}"),
        ("subject-cwd", "tree", f"share/v{revision + 20}/value"),
        ("probe", "tool", f"flyline-v{revision}-{PLATFORM}"),
        ("probe", "library", f"libflyline.so.{revision + 10}"),
        ("probe", "tree", f"share/v{revision + 20}"),
        ("probe", "tree", "."),
        ("retain", "library", f"libflyline.so.{revision + 10}"),
        ("retain", "tree", f"share/v{revision + 20}"),
    }
    assert expected <= {(row["consumer"], row["output"], row["selector"]) for row in rows}, rows
    for row in rows:
        assert row["span"]["file"].endswith("selectors.ts"), row
        assert row["span"]["line"] > 0, row
        assert "{version" not in row["selector"], row
    assert server.requests == before, "preview must not resolve or realize artifacts"
    print(f"artifact plan v{revision}: " + json.dumps(sorted(expected)))



def assert_environment(repo, revision):
    # No task/check references can accidentally pull this environment-only
    # library into the selected closure: packages contains only the shell.
    result = run(repo, "run", "--env", "tools", "--", "/bin/sh", "-c",
                 '/bin/cat "$LIBRARY"; printf "environment-literal:%s\\n" "$LITERAL"')
    assert f"library:v{revision + 10}\n" in result.stdout, result.stdout + result.stderr
    assert "environment-literal:{version}/{os}\n" in result.stdout, result.stdout
    evidence(f"reference-only environment v{revision}", result)


def assert_probe(repo, revision):
    assert_environment(repo, revision)
    result = run(repo, "task", "run", "probe")
    for text in (f"tool:v{revision}\n", f"library:v{revision + 10}\n", f"tree:v{revision + 20}\n",
                 "literal:{version}/{os}:{version}/{os}\n", "whole-artifact\n",
                 "literal-{version}-{os}\n", "body:{version}/{os}\n"):
        assert text in result.stdout, result.stdout + result.stderr
    # Both package-subject checks and the reported test -f must execute, not merely admit.
    assert result.stdout.count(f"library:v{revision + 10}\n") >= 4, result.stdout
    assert result.stdout.count(f"tree:v{revision + 20}\n") >= 4, result.stdout
    evidence(f"typed selectors v{revision}", result)


def test_artifact_commands_follow_independent_locks_and_retain_old_hook_context(sandbox, releases):
    publish_rooted(releases, 1)
    repo = artifact_workspace(sandbox, releases)
    sources = {path.relative_to(repo): path.read_bytes() for path in repo.rglob("*.ts")}
    state = sandbox / ".local/share/gripsack"
    lock = repo / "gripsack.lock"
    run(repo, "update")
    first_lock = lock.read_bytes()
    assert not list((state / "store").glob("*workspace-package*/package.json"))
    assert_artifact_plan(repo, releases, 1)
    assert_probe(repo, 1)
    run(repo, "apply", "retained")
    first_generation = (state / "current").readlink()
    effect = sandbox / "hook-effects"
    assert effect.read_text().splitlines() == ["library:v11", "tree:v21"]

    publish_rooted(releases, 2)
    before = list(releases.requests)
    assert_artifact_plan(repo, releases, 1)
    assert_probe(repo, 1)
    assert releases.requests == before
    assert lock.read_bytes() == first_lock

    run(repo, "update")
    assert lock.read_bytes() != first_lock
    assert_artifact_plan(repo, releases, 2)
    assert_probe(repo, 2)
    run(repo, "apply", "retained")
    assert (state / "current").readlink() != first_generation
    assert effect.read_text().splitlines() == ["library:v11", "tree:v21", "library:v12", "tree:v22"]

    releases.close()
    before = list(releases.requests)
    second_lock = lock.read_bytes()
    assert_artifact_plan(repo, releases, 2)
    assert_probe(repo, 2)
    evidence("offline rollback to old concrete hook", run(repo, "rollback", first_generation.name))
    assert effect.read_text().splitlines() == [
        "library:v11", "tree:v21", "library:v12", "tree:v22", "library:v11", "tree:v21",
    ]
    assert releases.requests == before
    assert lock.read_bytes() == second_lock
    assert {path.relative_to(repo): path.read_bytes() for path in repo.rglob("*.ts")} == sources


@pytest.mark.parametrize("position", ["argv", "env", "cwd", "environment", "subject"])
@pytest.mark.parametrize(("selector", "reason"), [
    ("{version}", "version"), ("{unsupported}", "unsupported"),
])
def test_artifact_tokens_fail_with_provenance_without_version_authority(sandbox, position, selector, reason):
    reference = f'artifact("data", {json.dumps(selector)})'
    argv = reference if position == "argv" else 'lit("/usr/bin/true")'
    fields = f', {position}:' + (f'{{VALUE:{reference}}}' if position == "env" else reference) if position in ("env", "cwd") else ""
    env = f', env:{{VALUE:{reference}}}' if position == "environment" else ""
    check = f'check("subject", {{subject:"data", run:exec({{argv:[lit("/bin/cat"), sourcePath({json.dumps(selector)})]}})}}),' if position == "subject" else ""
    checks = ', checks:["subject"]' if position == "subject" else ""
    repo = make_workspace_repo(sandbox / "invalid-artifact", {"invalid": f'''import {{
  pkg, provider, fileFetch, environment, profile, task, check, exec, lit, artifact, sourcePath,
}} from "@gripsack/core";
const target = {{os:"linux",arch:"x86_64"}} as const;
export default [
  pkg("data", {{producer:provider(fileFetch("payload")), commands:{{}}, target, layout:{{kind:"relocatable"}}}}),
  environment("tools", {{packages:[], target{env}}}),
  {check}
  task("probe", {{environment:"tools", steps:[exec({{argv:[{argv}]{fields}}})]{checks}}}),
  profile("selected", {{environment:"tools"}}),
];'''})
    (repo / "payload").mkdir()
    (repo / "payload/value").write_text("no release version\n")
    for args in (("plan", "--json"), ("task", "run", "probe")):
        result = grip(*args, cwd=repo)
        diagnostic = result.stdout + result.stderr
        assert result.returncode != 0, diagnostic
        assert reason in diagnostic.lower(), diagnostic
        assert "invalid.ts" in diagnostic, diagnostic
    assert not (sandbox / ".local/share/gripsack/current").exists()


@pytest.mark.parametrize("position", ["argv", "env", "cwd"])
def test_unsafe_artifact_selector_never_executes_consumer(sandbox, position):
    selector = 'artifact("data", "{os}/../../outside")'
    argv = f'lit("/usr/bin/true"), {selector}' if position == "argv" else 'lit("/usr/bin/true")'
    field = f", env:{{VALUE:{selector}}}" if position == "env" else (
        f", cwd:{selector}" if position == "cwd" else ""
    )
    repo = make_workspace_repo(sandbox / "unsafe-artifact", {"unsafe": f'''import {{
  pkg, provider, fileFetch, profile, task, exec, lit, artifact,
}} from "@gripsack/core";
export default [
  pkg("data", {{producer:provider(fileFetch("payload")), commands:{{}},
    target:{{os:"linux",arch:"x86_64"}}, layout:{{kind:"relocatable"}}}}),
  task("probe", {{steps:[exec({{argv:[{argv}]{field}}})]}}),
  profile("selected", {{}}),
];'''})
    (repo / "payload").mkdir()
    (repo / "payload/value").write_text("contained\n")
    for args in (("plan", "--json"), ("task", "run", "probe")):
        result = grip(*args, cwd=repo)
        diagnostic = result.stdout + result.stderr
        assert result.returncode != 0, diagnostic
        assert "unsafe.ts" in diagnostic, diagnostic
        assert any(word in diagnostic.lower() for word in ("relative", "parent", "escape", "segment")), diagnostic
