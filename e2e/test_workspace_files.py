"""Native captured/rendered files, shared blocks and retained-state recovery (A2)."""
import json

from conftest import grip


IMPORTS = '''import {
  defineWorkspace, workspace, profile, file, repoFile, templateText,
  literalText, symlinkTo, trackedCopyTo, managedBlock,
} from "@gripsack/core";
'''


def declare(repo, outputs):
    repo.mkdir(exist_ok=True)
    (repo / "gripsack.ts").write_text(
        IMPORTS + "export default defineWorkspace(() => workspace({ outputs: [\n"
        + outputs + "\n] }));\n"
    )


def run(repo, *args):
    result = grip(*args, cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def manifest(sandbox):
    return json.loads((sandbox / ".local/share/gripsack/current/manifest.json").read_text())


def rendered_profile(value, link="~/.linked"):
    content = f'source: repoFile("template.txt"), content: templateText("value={{{{ value }}}}\\n", {{ value: {json.dumps(value)} }})'
    return f'''profile("files", {{ files: [
      file({{ {content}, destination: symlinkTo({json.dumps(link)}) }}),
      file({{ {content}, destination: trackedCopyTo("~/.copied") }}),
      file({{ {content}, destination: managedBlock("~/.shared-rc", "settings") }}),
    ] }})'''


def test_rendered_content_composes_with_all_ownership_policies_and_rolls_back(sandbox):
    repo = sandbox / "files-env"
    declare(repo, rendered_profile("one"))
    (repo / "template.txt").write_text("value={{ value }}\n")
    shared = sandbox / ".shared-rc"
    shared.write_bytes(b"FOREIGN\r\n")
    run(repo, "check")
    run(repo, "plan")
    run(repo, "update", "--check")
    assert not (sandbox / ".linked").exists()
    assert not (sandbox / ".local/share/gripsack/current").exists()
    run(repo, "apply")
    first = manifest(sandbox)
    source = first["modules"]["files"]["store_path"]
    assert (sandbox / ".linked").is_symlink()
    assert (sandbox / ".linked").read_text() == "value=one\n"
    assert (sandbox / ".copied").read_text() == "value=one\n"
    assert shared.read_bytes().startswith(b"FOREIGN\r\n")
    assert b"value=one" in shared.read_bytes()
    run(repo, "apply")
    assert manifest(sandbox)["number"] == first["number"]

    # A destination-only change does not change the content producer identity.
    declare(repo, rendered_profile("one", "~/.moved-link"))
    run(repo, "apply")
    assert manifest(sandbox)["modules"]["files"]["store_path"] == source
    assert not (sandbox / ".linked").exists()
    assert (sandbox / ".moved-link").read_text() == "value=one\n"

    declare(repo, rendered_profile("two", "~/.moved-link"))
    (repo / "template.txt").write_text("new captured source binding\n")
    run(repo, "apply")
    assert (sandbox / ".copied").read_text() == "value=two\n"
    run(repo, "rollback", str(first["number"]))
    assert (sandbox / ".linked").read_text() == "value=one\n"
    assert (sandbox / ".copied").read_text() == "value=one\n"
    assert b"value=one" in shared.read_bytes()
    assert b"value=two" not in shared.read_bytes()
    assert shared.read_bytes().startswith(b"FOREIGN\r\n")
    run(repo, "store-verify")


def blocks(left="left", right="right"):
    entries = [f'file({{ content: literalText({json.dumps(left)}), destination: managedBlock("~/.shared-rc", "left marker") }})']
    if right is not None:
        entries.append(f'file({{ content: literalText({json.dumps(right)}), destination: managedBlock("~/.shared-rc", "right") }})')
    return 'profile("blocks", { files: [' + ",".join(entries) + '] })'


def test_distinct_blocks_update_prune_and_restore_without_losing_foreign_bytes(sandbox):
    repo = sandbox / "blocks-env"
    declare(repo, blocks())
    shared = sandbox / ".shared-rc"
    shared.write_text("foreign header\n")
    run(repo, "apply")
    first = manifest(sandbox)["number"]
    original = shared.read_bytes()
    assert b"left\n" in original and b"right\n" in original
    declare(repo, blocks("changed", None))
    run(repo, "apply")
    changed = shared.read_bytes()
    assert b"changed\n" in changed and b"right\n" not in changed
    assert changed.startswith(b"foreign header\n")
    run(repo, "rollback", str(first))
    assert shared.read_bytes() == original
    run(repo, "gc")
    assert shared.read_bytes() == original


def test_shared_file_reports_every_owner_and_rejects_conflicting_retained_modes(sandbox):
    repo = sandbox / "shared-owners"
    declare(repo, ",".join(
        f'profile("{name}", {{ files: [file({{ content: literalText("{name}"), destination: managedBlock("~/.shared-rc", "{name}") }})] }})'
        for name in ("alpha", "beta")
    ))
    run(repo, "apply")
    owners = run(repo, "why-owns", "~/.shared-rc").stdout.splitlines()
    assert {line.split()[0] for line in owners} == {"alpha", "beta"}
    shared = sandbox / ".shared-rc"
    original = shared.read_bytes()
    state = manifest(sandbox)
    first = state["modules"]["alpha"]["entries"][0]["file_mode"]
    state["modules"]["beta"]["entries"][0]["file_mode"] = first ^ 0o100
    retained = sandbox / ".local/share/gripsack/current/manifest.json"
    retained.write_text(json.dumps(state))
    refused = grip("apply", cwd=repo)
    assert refused.returncode != 0
    assert shared.read_bytes() == original


def test_crash_after_two_blocks_recovers_the_original_host_file(sandbox, monkeypatch):
    repo = sandbox / "crash-env"
    declare(repo, blocks())
    shared = sandbox / ".shared-rc"
    original = b"foreign original\n"
    shared.write_bytes(original)
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "workspace-file:2")
    crashed = grip("apply", cwd=repo)
    assert crashed.returncode != 0
    assert b"left\n" in shared.read_bytes() and b"right\n" in shared.read_bytes()
    assert not (sandbox / ".local/share/gripsack/current").exists()
    journal = sandbox / ".local/share/gripsack/journal"
    assert journal.stat().st_mode & 0o7777 == 0o700
    assert all(path.stat().st_mode & 0o7777 == 0o600 for path in journal.glob("*.json"))
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    monkeypatch.setenv("GRIPSACK_FS_RECOVER_ONLY", "1")
    run(repo, "apply")
    assert shared.read_bytes() == original
    assert not list(journal.glob("*.json"))


def test_recovery_io_failure_does_not_quarantine_an_admitted_record(sandbox, monkeypatch):
    repo = sandbox / "recovery-io-env"
    declare(repo, blocks())
    destination = sandbox / ".shared-rc"
    original = b"original\n"
    destination.write_bytes(original)
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "workspace-file:2")
    assert grip("apply", cwd=repo).returncode != 0
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    interrupted = destination.read_bytes()
    journal = sandbox / ".local/share/gripsack/journal"
    records = {path.name: path.read_bytes() for path in journal.glob("*.json")}
    record_name = next(name for name in records if name != "run.json")

    def reset_interrupted_state():
        destination.write_bytes(interrupted)
        for name, contents in records.items():
            (journal / name).write_bytes(contents)
            (journal / name).chmod(0o600)
        (journal / record_name).chmod(0o666)

    reset_interrupted_state()
    trace = sandbox / "recovery.trace"
    monkeypatch.setenv("GRIPSACK_FS_RECOVER_ONLY", "1")
    monkeypatch.setenv("GRIPSACK_FS_TRACE", str(trace))
    run(repo, "apply")
    assert destination.read_bytes() == original
    points = [line.split("\t", 3) for line in trace.read_text().splitlines()]
    cut = next(row[0] for row in points if row[1:3] == ["After", "FileSync"]
               and json.loads(row[3]).endswith(record_name))
    reset_interrupted_state()
    monkeypatch.setenv("GRIPSACK_FS_CUT", cut)
    failed = grip("apply", cwd=repo)
    assert failed.returncode != 0 and "injected After FileSync" in failed.stderr
    assert destination.read_bytes() == interrupted
    assert (journal / record_name).read_bytes() == records[record_name]
    assert not list((journal / "quarantine").glob("*.json"))
    monkeypatch.delenv("GRIPSACK_FS_CUT")
    run(repo, "apply")
    assert destination.read_bytes() == original


def test_scoped_profile_apply_keeps_the_other_profile_selection(sandbox):
    repo = sandbox / "scoped-env"
    def outputs(a, b):
        return ",".join(
            f'profile("{name}", {{ files: [file({{ content: literalText({json.dumps(value)}), destination: trackedCopyTo("~/.{name}") }})] }})'
            for name, value in [("alpha", a), ("beta", b)]
        )
    declare(repo, outputs("a1", "b1"))
    run(repo, "apply")
    beta = manifest(sandbox)["modules"]["beta"]
    declare(repo, outputs("a2", "b2"))
    run(repo, "plan", "alpha")
    run(repo, "apply", "alpha")
    assert (sandbox / ".alpha").read_text() == "a2"
    assert (sandbox / ".beta").read_text() == "b1"
    assert manifest(sandbox)["modules"]["beta"] == beta


def test_failed_render_and_physical_alias_fail_before_destination_mutation(sandbox):
    repo = sandbox / "invalid-env"
    declare(repo, '''profile("invalid", { files: [
      file({ content: literalText("must not land"), destination: trackedCopyTo("~/.first") }),
      file({ source: repoFile("template.txt"), content: templateText("{{ missing }}", {}), destination: symlinkTo("~/.second") }),
    ] })''')
    (repo / "template.txt").write_text("source")
    result = grip("apply", cwd=repo)
    assert result.returncode != 0
    assert not (sandbox / ".first").exists()
    assert not (sandbox / ".second").exists()
    assert not (sandbox / ".local/share/gripsack/current").exists()

    real = sandbox / "real"
    real.mkdir()
    (sandbox / "alias").symlink_to(real, target_is_directory=True)
    declare(repo, '''profile("alpha", { files: [
      file({ content: literalText("one"), destination: trackedCopyTo("~/real/config") }),
    ] }), profile("beta", { files: [
      file({ content: literalText("two"), destination: trackedCopyTo("~/alias/config") }),
    ] })''')
    checked = grip("check", "--json", cwd=repo)
    assert checked.returncode != 0
    aliases = [item for item in json.loads(checked.stdout)["diagnostics"] if item["code"] == "E119"]
    assert len(aliases) == 1 and len(aliases[0]["labels"]) == 2
    assert not (real / "config").exists()
    selected = grip("apply", "alpha", cwd=repo)
    assert selected.returncode != 0 and "E119" in selected.stderr
    assert not (real / "config").exists()


def test_native_take_over_preserves_original_bytes_and_permissions_for_rollback(sandbox):
    repo = sandbox / "adoption-env"
    declare(repo, 'profile("personal", { files: [] })')
    run(repo, "apply")
    baseline = manifest(sandbox)["number"]
    target = sandbox / ".adopted"
    target.write_bytes(b"unmanaged original\n")
    target.chmod(0o640)
    declare(repo, '''profile("personal", { files: [
      file({ content: literalText("managed replacement\\n"), destination: trackedCopyTo("~/.adopted") }),
    ] })''')
    run(repo, "apply")  # ordinary tracked-copy drift is kept, not taken over
    assert target.read_bytes() == b"unmanaged original\n"
    assert target.stat().st_mode & 0o7777 == 0o640
    run(repo, "apply", "--take-over")
    assert target.read_bytes() == b"managed replacement\n"
    run(repo, "rollback", str(baseline))
    assert target.read_bytes() == b"unmanaged original\n"
    assert target.stat().st_mode & 0o7777 == 0o640
