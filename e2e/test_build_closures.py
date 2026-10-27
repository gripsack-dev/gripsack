"""Native package runtime closures retain exact artifacts across generations.

The deleted module build-role/PATH/export tests have no workspace analogue:
BuildKit recipe execution is tested by its builder harness. These cases use
real offline downloaded packages, never a host-shell substitute for a builder.
"""
import json

from conftest import grip, make_workspace_repo, make_tarball, remove_output


def run(repo, *args):
    result = grip(*args, cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def manifest(sandbox):
    wire = json.loads((sandbox / ".local/share/gripsack/current/manifest.json").read_text())
    return wire.get("generation", wire)


def package(name, archive, runtime=()):
    return f'''import {{pkg,provider,tarball}} from "@gripsack/core";
export default pkg({json.dumps(name)}, {{producer:provider(tarball({json.dumps(archive.as_uri())})),
 commands:{{}},runtime:{json.dumps(list(runtime))},
 target:{{os:"linux",arch:"x86_64"}},layout:{{kind:"relocatable"}}}});'''


def fixture(sandbox):
    library = make_tarball(sandbox / "runtime.tar.gz", {"lib/value": b"runtime-v1\n"})
    application = make_tarball(sandbox / "app.tar.gz", {"share/value": b"app-v1\n"})
    repo = make_workspace_repo(sandbox / "env", {
        "runtime": package("runtime", library),
        "app": package("app", application, ["runtime"]),
        "personal": '''import {environment,profile,file,artifactFile,identity,trackedCopyTo} from "@gripsack/core";
export default [environment("dev", {packages:["app"],target:{os:"linux",arch:"x86_64"}}),
 profile("personal", {environment:"dev",files:[file({source:artifactFile("app","share/value"),
 content:identity(),destination:trackedCopyTo("~/.application")})]})];''',
        "unrelated": '''import {profile,file,literalText,trackedCopyTo} from "@gripsack/core";
export default profile("unrelated", {files:[file({content:literalText("untouched"),
 destination:trackedCopyTo("~/.unrelated")})]});''',
    })
    return repo, library, application


def retained_paths(sandbox):
    return {path.parent for path in (sandbox / ".local/share/gripsack/store").glob("*/lib/value")}


def test_native_runtime_closure_survives_update_rollback_and_collection(sandbox):
    repo, library, application = fixture(sandbox)
    run(repo, "update")
    run(repo, "apply")
    first = manifest(sandbox)
    roots = retained_paths(sandbox)
    assert roots, "runtime dependency must be acquired even without a deployed command"
    assert all((path / "value").read_bytes() == b"runtime-v1\n" for path in roots)
    assert (sandbox / ".application").read_bytes() == b"app-v1\n"
    assert not (sandbox / ".local/bin/runtime").exists()
    run(repo, "apply")
    assert manifest(sandbox)["number"] == first["number"]

    make_tarball(library, {"lib/value": b"runtime-v2\n"})
    make_tarball(application, {"share/value": b"app-v2\n"})
    run(repo, "update", "personal")
    run(repo, "apply", "personal")
    second = manifest(sandbox)
    assert second["modules"]["unrelated"] == first["modules"]["unrelated"]
    assert (sandbox / ".unrelated").read_text() == "untouched"
    assert (sandbox / ".application").read_bytes() == b"app-v2\n"
    newer = retained_paths(sandbox) - roots
    assert newer
    assert all((path / "value").read_bytes() == b"runtime-v2\n" for path in newer)
    run(repo, "gc")
    assert all(path.exists() for path in roots | newer)
    run(repo, "rollback", str(first["number"]))
    assert (sandbox / ".application").read_bytes() == b"app-v1\n"
    run(repo, "rollback", str(second["number"]))
    assert (sandbox / ".application").read_bytes() == b"app-v2\n"
    run(repo, "store-verify")
    assert not (sandbox / ".local/share/gripsack/buildkit").exists()

    for output in ("personal", "app", "runtime"):
        remove_output(repo, output)
    run(repo, "apply")
    assert not (sandbox / ".application").exists()
    run(repo, "gc")
    assert all(path.exists() for path in roots | newer)
    config = sandbox / ".config/gripsack"
    config.mkdir(parents=True, exist_ok=True)
    (config / "config.toml").write_text("[settings]\nkeep_generations = 1\n")
    run(repo, "gc")
    assert all(not path.exists() for path in roots | newer)
    assert (sandbox / ".unrelated").read_text() == "untouched"
