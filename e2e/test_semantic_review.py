"""Offline semantic review uses admitted facts, never evaluator or host effects."""
import json
import os
from pathlib import Path

from conftest import GRIP, grip, run_grip


def declare(repo, text, destination="~/.semantic-config"):
    repo.mkdir(exist_ok=True)
    (repo / "gripsack.ts").write_text(
        'import {defineWorkspace,workspace,profile,file,literalText,trackedCopyTo} '
        'from "@gripsack/core";\n'
        'export default defineWorkspace(() => workspace({outputs:[profile("config", '
        '{files:[file({content:literalText(' + json.dumps(text) + '),'
        'destination:trackedCopyTo(' + json.dumps(destination) + ')})]})]}));\n'
    )


def successful(repo, *args, **kwargs):
    result = grip(*args, cwd=repo, **kwargs)
    assert result.returncode == 0, result.stdout + result.stderr
    return result


def test_retained_comparison_is_offline_private_and_preserves_unknowns(sandbox, monkeypatch):
    repo = sandbox / "semantic-repo"
    declare(repo, "before\n")
    successful(repo, "apply")
    before = sandbox / "before.snapshot.json"
    successful(repo, "snapshot", "current", "--out", str(before))
    assert before.stat().st_mode & 0o777 == 0o600
    recorded = json.loads(before.read_text())
    workspace = next(node for node in recorded["nodes"] if node["id"] == "workspace")
    for prefix in ["/facts", "/probes", "/definitions"]:
        evidence = [fact for fact in workspace["facts"] if fact["path"] == prefix or fact["path"].startswith(prefix + "/")]
        assert evidence, prefix
        assert all(fact["knowledge"]["state"] == "known" for fact in evidence), evidence
    assert not {"historical_declaration", "missing_probe", "missing_lock"} & set(recorded["completeness"]["unknown_reasons"])
    declare(repo, "after\n", "~/.semantic-moved")
    successful(repo, "apply")
    after = sandbox / "after.snapshot.json"
    successful(repo, "snapshot", "current", "--out", str(after))
    destination = sandbox / ".semantic-moved"
    current = sandbox / ".local/share/gripsack/current"
    selected = current.readlink()
    metadata = destination.stat()
    monkeypatch.setenv("GRIPSACK_DENO", str(sandbox / "no-evaluator"))
    result = grip("diff", str(before), str(after), "--json", cwd=repo, approve=False)
    # New retained declarations and observations are known. Generic private
    # content remains redacted, never a low-entropy secret hash.
    assert result.returncode == 3, result.stdout + result.stderr
    report = json.loads(result.stdout)
    assert report["changes"]
    assert any(change["kind"] == "deployment" and change["difference"] == "changed" for change in report["changes"])
    assert not report["completeness"]["complete"]
    assert report["completeness"]["unknown_reasons"]
    assert current.readlink() == selected
    assert destination.read_text() == "after\n"
    assert destination.stat().st_mtime_ns == metadata.st_mtime_ns
    same = grip("diff", str(before), str(before), "--json", cwd=repo, approve=False)
    assert same.returncode == 3, same.stdout + same.stderr
    assert not json.loads(same.stdout)["completeness"]["complete"]
    # Rollback restores the selected frozen review record; it neither imports
    # today's declarations nor needs the evaluator or surviving source checkout.
    (repo / "gripsack.ts").unlink()
    successful(repo, "rollback", "1", approve=False)
    restored = sandbox / "restored.snapshot.json"
    successful(repo, "snapshot", "current", "--out", str(restored), approve=False)
    assert json.loads(restored.read_text()) == json.loads(before.read_text())
    assert (sandbox / ".semantic-config").read_text() == "before\n"


def test_import_rejects_duplicate_fields_without_evaluation(sandbox, monkeypatch):
    snapshot = sandbox / "hostile.json"
    snapshot.write_text('{"version":1,"version":1}')
    monkeypatch.setenv("GRIPSACK_DENO", str(sandbox / "missing-deno"))
    result = grip("diff", str(snapshot), str(snapshot), "--json", cwd=sandbox, approve=False)
    assert result.returncode == 1
    assert not (sandbox / ".local/share/gripsack/current").exists()


def test_proposed_ir_rejects_original_top_level_and_nested_duplicates(sandbox, monkeypatch):
    ir = {
        "ir_version": 10,
        "host": {"os": "linux", "arch": "x86_64", "tags": []},
        "workspace": {"span": {"file": "gripsack.ts", "line": 1}, "outputs": [{
            "kind": "profile", "name": "plain",
            "span": {"file": "gripsack.ts", "line": 2}, "files": [],
        }]},
    }
    encoded = json.dumps(ir)
    proposal = sandbox / "proposal.json"
    proposal.write_text(encoded)
    monkeypatch.setenv("GRIPSACK_DENO", str(sandbox / "missing-evaluator"))
    successful(sandbox, "snapshot", f"proposed:{proposal}", "--out",
               str(sandbox / "positive.snapshot.json"), approve=False)
    ambiguous = [
        '{"ir_version":10,' + encoded[1:],
        encoded.replace('"name": "plain"', '"name": "shadow", "name": "plain"'),
    ]
    for index, text in enumerate(ambiguous):
        proposal.write_text(text)
        target = sandbox / f"refused-{index}.json"
        result = grip("snapshot", f"proposed:{proposal}", "--out", str(target),
                      cwd=sandbox, approve=False)
        assert result.returncode == 1, result.stdout + result.stderr
        assert not target.exists()


def test_new_snapshot_metadata_preserves_repeated_apply_and_span_only_noop(sandbox):
    repo = sandbox / "semantic-noop"
    declare(repo, "stable\n")
    successful(repo, "apply")
    home = sandbox / ".local/share/gripsack"
    current = home / "current"
    first_selection = current.readlink()
    first_manifest = (current / "manifest.json").read_bytes()
    destination = sandbox / ".semantic-config"
    first_identity = (destination.stat().st_ino, destination.stat().st_mtime_ns)
    before = sandbox / "original.snapshot.json"
    successful(repo, "snapshot", "current", "--out", str(before))
    successful(repo, "apply")
    assert current.readlink() == first_selection
    source = repo / "gripsack.ts"
    source.write_text("// source locations moved, meaning did not\n\n" + source.read_text())
    successful(repo, "apply")
    assert current.readlink() == first_selection
    assert (current / "manifest.json").read_bytes() == first_manifest
    assert (destination.stat().st_ino, destination.stat().st_mtime_ns) == first_identity
    after = sandbox / "after-span.snapshot.json"
    successful(repo, "snapshot", "current", "--out", str(after))
    assert json.loads(after.read_text()) == json.loads(before.read_text())
    manifest = json.loads(first_manifest)
    manifest = manifest.get("generation", manifest)
    roots = [path for state in manifest["modules"].values() for path in state.get("build_closure", [])]
    assert any(path.endswith("-semantic-review") for path in roots)
    successful(repo, "store-verify")


def test_frozen_pre_semantic_generation_remains_partial_without_source(sandbox, monkeypatch):
    # Exercise the retained legacy wire directly, not a current snapshot with
    # fields deleted. This pre-0.51 manifest has no semantic artifact reference.
    fixture = Path(__file__).parent / "fixtures/semantic/pre-semantic-generation-v1.json"
    manifest = json.loads(fixture.read_text())
    home = sandbox / ".local/share/gripsack"
    legacy_store = home / "store/legacy-profile"
    legacy_store.mkdir(parents=True)
    manifest["modules"]["legacy-profile"]["store_path"] = str(legacy_store)
    generation = home / "generations/1"
    generation.mkdir(parents=True)
    (generation / "manifest.json").write_text(json.dumps(manifest))
    (home / "current").symlink_to("generations/1")
    monkeypatch.setenv("GRIPSACK_DENO", str(sandbox / "missing-evaluator"))
    snapshot = sandbox / "legacy.snapshot.json"
    successful(sandbox, "snapshot", "current", "--out", str(snapshot), approve=False)
    document = json.loads(snapshot.read_text())
    assert not document["completeness"]["complete"]
    assert {"historical_declaration", "missing_probe", "missing_lock"} <= set(document["completeness"]["unknown_reasons"])
    comparison = grip("diff", str(snapshot), str(snapshot), "--json", cwd=sandbox, approve=False)
    assert comparison.returncode == 3, comparison.stdout + comparison.stderr


def test_configuration_view_omits_values_and_does_not_rewrite(sandbox):
    before = sandbox / "before.json"
    after = sandbox / "after.json"
    before.write_text('{"visible":"old-private-value","password":"old-password"}')
    after.write_text('{"visible":"new-private-value","password":"new-password"}')
    result = grip("diff-config", str(before), str(after), "--format", "json", "--json",
                  cwd=sandbox, approve=False)
    assert result.returncode == 2, result.stdout + result.stderr
    json.loads(result.stdout)
    assert "old-private-value" not in result.stdout
    assert "new-private-value" not in result.stdout
    assert "old-password" not in result.stdout
    assert "new-password" not in result.stdout
    assert before.read_text().startswith('{"visible":"old-private-value"')
    assert after.read_text().startswith('{"visible":"new-private-value"')


def test_duplicate_config_fallback_and_terminal_json_agree(sandbox):
    before = sandbox / "duplicate.json"
    after = sandbox / "valid.json"
    before.write_text('{"key":1,"key":2}')
    after.write_text('{"key":2}')
    args = ("diff-config", str(before), str(after), "--format", "json")
    structured = grip(*args, "--json", cwd=sandbox, approve=False)
    terminal = grip(*args, cwd=sandbox, approve=False)
    assert structured.returncode == terminal.returncode == 3
    report = json.loads(structured.stdout)
    assert json.loads(terminal.stdout.split("\n", 1)[1]) == report
    assert report["structural_equal"] is None
    assert report["fallback"]["before_reason"] == "duplicate_key"
    assert not report["complete"]


def test_review_refuses_fifo_without_waiting_for_writer(sandbox):
    fifo = sandbox / "not-a-snapshot"
    os.mkfifo(fifo)
    result = run_grip([str(GRIP.resolve()), "diff", str(fifo), str(fifo)],
                      cwd=sandbox, approve=False, capture_output=True, text=True, timeout=5)
    assert result.returncode == 1


def test_plan_json_explains_actual_proposed_bytes_without_mutation(sandbox):
    repo = sandbox / "plan-config-repo"
    declare(repo, '{"enabled":true}\n', "~/.semantic.json")
    successful(repo, "apply")
    destination = sandbox / ".semantic.json"
    before = destination.read_bytes()
    declare(repo, '{"enabled":false}\n', "~/.semantic.json")
    result = successful(repo, "plan", "--json")
    report = json.loads(result.stdout)
    operations = [operation for operation in report["operations"] if operation["kind"] == "write"]
    assert len(operations) == 1
    view = operations[0]["configuration"]
    assert view["scope"] == "whole_file"
    assert view["exact_bytes_equal"] is False
    assert view["structural_equal"] is False
    assert view["changes"][0]["path"] == '$["enabled"]'
    assert view["changes"][0]["after"]["disclosure"] == "omitted_portable"
    assert not report["execution_authority"]
    assert destination.read_bytes() == before


def test_plan_keeps_managed_block_and_foreign_observations_separate(sandbox):
    repo = sandbox / "block-review-repo"
    repo.mkdir()

    def block(value):
        (repo / "gripsack.ts").write_text(
            'import {defineWorkspace,workspace,profile,file,literalText,managedBlock} '
            'from "@gripsack/core";\n'
            'export default defineWorkspace(() => workspace({outputs:[profile("config", '
            '{files:[file({content:literalText(' + json.dumps(f"enabled = {value}\n") + '),'
            'destination:managedBlock("~/.semantic.toml","settings")})]})]}));\n'
        )

    destination = sandbox / ".semantic.toml"
    foreign = 'foreign = "untouched"\n'
    destination.write_text(foreign)
    block("true")
    successful(repo, "apply")
    before = destination.read_bytes()
    block("false")
    report = json.loads(successful(repo, "plan", "--json").stdout)
    operation = next(op for op in report["operations"] if op["kind"] == "managed_block")
    assert operation["configuration_scope"] == "managed_block"
    assert operation["managed_blocks_observed"] == 1
    assert operation["foreign_bytes_observed"] == len(foreign)
    assert not operation["foreign_drift_known"]
    assert operation["configuration"]["structural_equal"] is False
    assert destination.read_bytes() == before
