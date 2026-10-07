"""Real native effects and durable identity across the activation crash windows."""
import json
import os
import shlex
import shutil
import subprocess
import time

import pytest
from conftest import GRIP, grip, make_env_repo, remove_module, run_grip, start_grip


def fixture_repo(sandbox, script, *, trigger="post_activate", second=None):
    declarations = [f"customHook({json.dumps(script)}, {json.dumps(trigger)})"]
    if second is not None:
        declarations.append(f"customHook({json.dumps(second)})")
    repo = make_env_repo(sandbox / "env", {"demo": f'''
import {{ module, trackedCopy, customHook }} from "@gripsack/core";
export default module("demo", {{
  config: {{ payload: trackedCopy("~/.owned") }},
  activate: [{", ".join(declarations)}],
}});
'''})
    (repo / "payload").write_text("owned bytes\n")
    return repo


def outcomes(repo):
    result = grip("hooks", "list", "--json", cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    return json.loads(result.stdout)["intents"]


def append_script(path):
    return ("printf '%s:%s\\n' \"$GRIPSACK_ACTIVATION_INTENT_ID\" "
            f'"$GRIPSACK_ACTIVATION_ATTEMPT" >> {shlex.quote(str(path))}')


@pytest.mark.parametrize("cut,observed_before,expected_attempt,expected_effects", [
    ("hook-after-start", 0, 2, 1),
    ("hook-after-effect", 1, 2, 2),
    ("hook-after-receipt", 1, 1, 1),
    ("hooks-before-cleanup", 1, 1, 1),
])
def test_replay_preserves_token_and_skips_only_durable_terminal_results(
        sandbox, monkeypatch, cut, observed_before, expected_attempt, expected_effects):
    effect = sandbox / "effects"
    repo = fixture_repo(sandbox, append_script(effect))
    monkeypatch.setenv("GRIPSACK_ACTIVATION_INTENT_ID", "forged-ambient-id")
    monkeypatch.setenv("GRIPSACK_ACTIVATION_ATTEMPT", "999")
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", cut)
    interrupted = grip("apply", "--host", "testhost", cwd=repo)
    assert interrupted.returncode != 0
    before = effect.read_text().splitlines() if effect.exists() else []
    assert len(before) == observed_before
    first = outcomes(repo)
    assert len(first) == 1 and first[0]["pending"]
    identity = first[0]["intent"]
    assert identity != "forged-ambient-id"
    expected_state = "started" if cut in {"hook-after-start", "hook-after-effect"} else "succeeded"
    assert first[0]["state"] == {"kind": expected_state, "attempt": 1}
    for line in before:
        assert line == f"{identity}:1"
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    recovered = grip("apply", "--host", "testhost", cwd=repo)
    assert recovered.returncode == 0, recovered.stdout + recovered.stderr
    after = effect.read_text().splitlines()
    assert len(after) == expected_effects
    assert after[-1] == f"{identity}:{expected_attempt}"
    final = outcomes(repo)
    assert len(final) == 1
    assert final[0]["intent"] == identity and not final[0]["pending"]
    assert final[0]["state"] == {"kind": "succeeded", "attempt": expected_attempt}
    assert (sandbox / ".owned").read_text() == "owned bytes\n"
    assert not (sandbox / ".local/share/gripsack/activation.json").exists()
    # Diagnostic retention is not recovery/outcome retention.
    runs = sandbox / ".local/share/gripsack/runs"
    if runs.exists():
        shutil.rmtree(runs)
    assert not runs.exists()
    assert outcomes(repo) == final


def test_crash_between_hooks_does_not_repeat_completed_effect(sandbox, monkeypatch):
    first, second = sandbox / "first", sandbox / "second"
    repo = fixture_repo(sandbox, append_script(first), second=append_script(second))
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "hook-after-receipt")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode != 0
    assert first.exists() and not second.exists()
    pending = outcomes(repo)
    assert [row["state"]["kind"] for row in pending] == ["succeeded", "pending"]
    first_bytes = first.read_bytes()
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    resumed = grip("apply", "--host", "testhost", cwd=repo)
    assert resumed.returncode == 0, resumed.stdout + resumed.stderr
    assert first.read_bytes() == first_bytes
    assert second.read_text().splitlines() == [f'{pending[1]["intent"]}:1']
    assert {row["state"]["kind"] for row in outcomes(repo)} == {"succeeded"}


def test_known_failure_is_archived_without_retry_or_rollback(sandbox):
    effect = sandbox / "failed-effect"
    repo = fixture_repo(sandbox, append_script(effect) + "; exit 9")
    first = grip("apply", "--host", "testhost", cwd=repo)
    assert first.returncode == 0, first.stdout + first.stderr
    original = effect.read_bytes()
    failed = outcomes(repo)
    assert failed[0]["state"] == {"kind": "failed", "attempt": 1}
    assert failed[0]["processes"][0]["exit_code"] == 9
    assert (sandbox / ".owned").read_text() == "owned bytes\n"
    satisfied = grip("apply", "--host", "testhost", cwd=repo)
    assert satisfied.returncode == 0, satisfied.stdout + satisfied.stderr
    assert effect.read_bytes() == original
    assert outcomes(repo) == failed
    # A deliberate new rollback instance is new delivery, even for generation 1.
    rollback = grip("rollback", "1", cwd=repo)
    assert rollback.returncode == 0, rollback.stdout + rollback.stderr
    repeated = outcomes(repo)
    assert len({row["activation"] for row in repeated}) == 2
    assert len({row["intent"] for row in repeated}) == 2
    assert all(row["state"] == {"kind": "failed", "attempt": 1} for row in repeated)
    assert len(effect.read_text().splitlines()) == 2
    assert (sandbox / ".local/share/gripsack/current").readlink().name == "1"


def test_pending_hook_evidence_blocks_gc_in_both_modes(sandbox, monkeypatch):
    effect = sandbox / "effect"
    repo = fixture_repo(sandbox, append_script(effect))
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "hook-after-start")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode != 0
    home = sandbox / ".local/share/gripsack"
    before = (home / "activation.json").read_bytes()
    for arguments in [[], ["--dry-run"]]:
        refused = grip("gc", *arguments, cwd=repo)
        assert refused.returncode != 0
        assert (home / "activation.json").read_bytes() == before
        assert (home / "generations/1/manifest.json").exists()
        assert not effect.exists()
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    collected = grip("gc", cwd=repo)
    assert collected.returncode == 0, collected.stdout + collected.stderr
    assert len(effect.read_text().splitlines()) == 1


def test_saved_removal_hook_replays_after_its_module_is_gone(sandbox, monkeypatch):
    effect = sandbox / "removed-effects"
    repo = fixture_repo(sandbox, append_script(effect), trigger="on_remove")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    assert not effect.exists()
    remove_module(repo, "demo")
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "hook-after-effect")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode != 0
    pending = outcomes(repo)
    identity = pending[0]["intent"]
    assert pending[0]["contributors"] == [{"module": "demo", "trigger": "on_remove"}]
    assert not (sandbox / ".owned").exists()
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    resumed = grip("apply", "--host", "testhost", cwd=repo)
    assert resumed.returncode == 0, resumed.stdout + resumed.stderr
    assert effect.read_text().splitlines() == [f"{identity}:1", f"{identity}:2"]


def test_missing_intent_record_never_becomes_fresh_attempt(sandbox, monkeypatch):
    effect = sandbox / "effects"
    repo = fixture_repo(sandbox, append_script(effect))
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "hook-after-effect")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode != 0
    row = outcomes(repo)[0]
    home = sandbox / ".local/share/gripsack"
    pointer = (home / "activation.json").read_bytes()
    state = home / "activation" / row["activation"] / "outcomes" / (row["intent"] + ".json")
    state.unlink()
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    refused = grip("apply", "--host", "testhost", cwd=repo)
    assert refused.returncode != 0
    assert effect.read_text().splitlines() == [f'{row["intent"]}:1']
    assert (home / "activation.json").read_bytes() == pointer


def test_identical_custom_declarations_have_distinct_delivery_identity(sandbox):
    effect = sandbox / "duplicates"
    script = append_script(effect)
    repo = fixture_repo(sandbox, script, second=script)
    applied = grip("apply", "--host", "testhost", cwd=repo)
    assert applied.returncode == 0, applied.stdout + applied.stderr
    rows = outcomes(repo)
    assert len(rows) == 2 and rows[0]["intent"] != rows[1]["intent"]
    assert effect.read_text().splitlines() == [f'{row["intent"]}:1' for row in rows]
    assert rows[0]["action_sha256"] == rows[1]["action_sha256"]


def test_legacy_pending_migrates_before_first_replay_and_keeps_the_new_token(sandbox, monkeypatch):
    effect = sandbox / "legacy-effect"
    repo = fixture_repo(sandbox, ":")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    home = sandbox / ".local/share/gripsack"
    pointer = home / "activation.json"
    pointer.write_text(json.dumps({"generation": 1, "intents": [{
        "module": "legacy", "action": {"kind": "custom_shell", "script": append_script(effect)},
    }]}))
    original = pointer.read_bytes()
    legacy = [row for row in outcomes(repo) if row["pending"]]
    assert legacy[0]["activation"] is None and legacy[0]["intent"] is None
    assert legacy[0]["state"] == {"kind": "legacy_ambiguous"}
    assert pointer.read_bytes() == original and not effect.exists()
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "hook-after-effect")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode != 0
    migrated = [row for row in outcomes(repo) if row["pending"]][0]
    assert migrated["identity_origin"] == "legacy_identity_unavailable"
    assert effect.read_text().splitlines() == [f'{migrated["intent"]}:1']
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    resumed = grip("apply", "--host", "testhost", cwd=repo)
    assert resumed.returncode == 0, resumed.stdout + resumed.stderr
    assert effect.read_text().splitlines() == [f'{migrated["intent"]}:1', f'{migrated["intent"]}:2']
    final = [row for row in outcomes(repo) if row["activation"] == migrated["activation"]][0]
    assert final["state"] == {"kind": "succeeded", "attempt": 2} and not final["pending"]


@pytest.mark.parametrize("option,append_effects", [("--duplicate", 2), ("--crash-after-start", 1)])
def test_fixture_simulations_never_select_or_modify_live_hook_state(sandbox, option, append_effects):
    home = sandbox / ".local/share/gripsack"
    generation = home / "generations/1"
    generation.mkdir(parents=True)
    (generation / "manifest.json").write_text('{"number":1,"modules":{}}')
    (home / "current").symlink_to("generations/1")
    sentinel = sandbox / "live-hook-must-not-run"
    pending = home / "activation.json"
    pending.write_text(json.dumps({"generation": 1, "intents": [{
        "module": "live", "action": {"kind": "custom_shell", "script": f"touch {shlex.quote(str(sentinel))}"},
    }]}))
    before = pending.read_bytes()
    result = grip("hooks", "test", option, cwd=sandbox)
    assert result.returncode == 0, result.stdout + result.stderr
    report = json.loads(result.stdout)
    assert report["fixture_only"] is True
    assert report["append_effects"] == append_effects and report["receiver_effects"] == 1
    assert len(report["outcomes"]) == 2
    assert all(row["succeeded"] and row["attempt"] == 2 for row in report["outcomes"])
    assert pending.read_bytes() == before and not sentinel.exists()
    assert (home / "current").readlink().as_posix() == "generations/1"
    assert not (home / "runs").exists(), "fixture-only dispatch must not initialize live tracing"


def test_hook_output_controls_are_escaped_and_script_values_stay_out_of_logs(sandbox):
    script = "private='DUMMY_HOOK_SECRET'; printf '\\033]0;forged\\007\\r\\033[2Jbody\\n'"
    repo = fixture_repo(sandbox, script)
    applied = grip("apply", "--host", "testhost", cwd=repo)
    assert applied.returncode == 0, applied.stdout + applied.stderr
    assert "body" in applied.stdout
    # Trusted renderer SGR colours are distinct from the hook's OSC/erase/BEL.
    assert not any(control in applied.stdout + applied.stderr
                   for control in ["\x1b]0;forged", "\x1b[2J", "\x07", "\r"])
    logs = sandbox / ".local/share/gripsack/runs"
    assert all("DUMMY_HOOK_SECRET" not in path.read_text() for path in logs.glob("*.jsonl"))
    inspected = grip("hooks", "list", "--json", cwd=repo)
    assert inspected.returncode == 0, inspected.stderr
    assert "DUMMY_HOOK_SECRET" not in inspected.stdout
    assert json.loads(inspected.stdout)["intents"][0]["state"]["kind"] == "succeeded"


def test_process_death_during_hook_keeps_ambiguous_delivery_identity(sandbox):
    started, release, effect = sandbox / "started", sandbox / "release", sandbox / "effect"
    body = f'''
import os, time
from pathlib import Path
Path({str(started)!r}).write_text("started")
deadline = time.monotonic() + 10
while not Path({str(release)!r}).exists():
    if time.monotonic() >= deadline:
        raise SystemExit(8)
    time.sleep(0.01)
with Path({str(effect)!r}).open("a") as output:
    output.write(os.environ["GRIPSACK_ACTIVATION_INTENT_ID"] + ":" + os.environ["GRIPSACK_ACTIVATION_ATTEMPT"] + "\\n")
'''
    repo = fixture_repo(sandbox, "python3 -c " + shlex.quote(body))
    process = start_grip([str(GRIP), "apply", "--host", "testhost"], cwd=repo,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        deadline = time.monotonic() + 20
        while not started.exists() and process.poll() is None and time.monotonic() < deadline:
            time.sleep(0.01)
        assert started.exists(), "the bounded fixture hook never started"
        pending = outcomes(repo)[0]
        assert pending["state"] == {"kind": "started", "attempt": 1}
        process.kill()
        # A killed supervisor is not a complete-tree containment promise. This
        # known bounded child may finish; replay must retain the same token.
        release.write_text("finish")
        process.communicate(timeout=20)
    finally:
        release.write_text("finish")
        if process.poll() is None:
            process.kill()
        process.communicate(timeout=20)
    # Waiting for grip reaps only grip, not the now-orphaned bounded fixture.
    deadline = time.monotonic() + 15
    expected = f'{pending["intent"]}:1\n'
    while (not effect.exists() or effect.read_text() != expected) and time.monotonic() < deadline:
        time.sleep(0.01)
    assert effect.read_text().splitlines() == [f'{pending["intent"]}:1']
    resumed = grip("apply", "--host", "testhost", cwd=repo)
    assert resumed.returncode == 0, resumed.stdout + resumed.stderr
    assert effect.read_text().splitlines() == [f'{pending["intent"]}:1', f'{pending["intent"]}:2']


@pytest.mark.parametrize("phase,fault", [
    ("outcome-sync", "error"), ("outcome-sync", "kill"), ("archive-before-clear", "kill"),
])
def test_outcome_durability_failure_retains_pending_authority(sandbox, monkeypatch, phase, fault):
    effect = sandbox / "durability-effects"
    repo = fixture_repo(sandbox, append_script(effect))
    assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    trace = sandbox / "hook-boundaries.tsv"
    with monkeypatch.context() as recording:
        recording.setenv("GRIPSACK_FS_TRACE", str(trace))
        trained = grip("rollback", "1", cwd=repo)
    assert trained.returncode == 0, trained.stdout + trained.stderr
    rows = [line.split("\t", 3) for line in trace.read_text().splitlines()]
    if phase == "archive-before-clear":
        cut = next(int(row[0]) for row in rows
                   if row[1:] == ["Before", "Unlink", '"activation.json"'])
    else:
        publications = {}
        for index, row in enumerate(rows):
            if row[1:3] == ["After", "FilePublish"]:
                name = json.loads(row[3])
                if len(name) == 69 and name.endswith(".json"):
                    publications.setdefault(name, []).append(index)
        state_writes = [indices for indices in publications.values() if len(indices) == 3]
        assert len(state_writes) == 1, publications
        after_outcome = state_writes[0][-1]
        cut = next(int(row[0]) for row in rows[after_outcome + 1:]
                   if row[1:3] == ["Before", "DirSync"])
    home = sandbox / ".local/share/gripsack"
    previous = (home / "current").readlink()
    with monkeypatch.context() as crashing:
        crashing.setenv("GRIPSACK_FS_CUT", str(cut))
        crashing.setenv("GRIPSACK_FS_FAULT", fault)
        interrupted = grip("rollback", "1", cwd=repo)
    assert (interrupted.returncode != 0) == (fault == "kill"), interrupted.stdout + interrupted.stderr
    assert (home / "current").readlink() != previous
    pending = [row for row in outcomes(repo) if row["pending"]]
    assert len(pending) == 1 and pending[0]["state"] == {"kind": "succeeded", "attempt": 1}
    assert (home / "activation.json").exists()
    before = effect.read_bytes()
    resumed = grip("apply", "--host", "testhost", cwd=repo)
    assert resumed.returncode == 0, resumed.stdout + resumed.stderr
    assert effect.read_bytes() == before, "an observed terminal result must be sealed, not rerun"
    assert not (home / "activation.json").exists()


def test_replay_uses_saved_action_not_changed_repository(sandbox, monkeypatch):
    original, replacement = sandbox / "original", sandbox / "replacement"
    repo = fixture_repo(sandbox, append_script(original))
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "hook-after-effect")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode != 0
    identity = outcomes(repo)[0]["intent"]
    module = repo / "modules/demo.ts"
    source = module.read_text()
    module.write_text(source.replace(json.dumps(append_script(original)),
                                     json.dumps(f"touch {shlex.quote(str(replacement))}")))
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    monkeypatch.setenv("GRIPSACK_FS_RECOVER_ONLY", "1")
    resumed = grip("apply", "--host", "testhost", cwd=repo)
    assert resumed.returncode == 0, resumed.stdout + resumed.stderr
    assert original.read_text().splitlines() == [f"{identity}:1", f"{identity}:2"]
    assert not replacement.exists()


def test_authoritative_hook_records_stay_private_under_permissive_umask(sandbox):
    import resource

    repo = fixture_repo(sandbox, ":")

    def permissive_child():
        os.umask(0)
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))

    interrupted = run_grip([str(GRIP), "apply", "--host", "testhost"], cwd=repo,
    env={**os.environ, "GRIPSACK_CRASH_AFTER": "hook-after-start"},
    preexec_fn=permissive_child, capture_output=True, text=True, timeout=30,)
    assert interrupted.returncode != 0
    home = sandbox / ".local/share/gripsack"
    assert (home / "activation.json").stat().st_mode & 0o7777 == 0o600
    records = home / "activation"
    assert records.stat().st_mode & 0o7777 == 0o700
    for path in records.rglob("*"):
        assert not path.is_symlink()
        assert path.stat().st_mode & 0o7777 == (0o700 if path.is_dir() else 0o600)
    assert outcomes(repo)[0]["state"] == {"kind": "started", "attempt": 1}


@pytest.mark.parametrize("corruption", [
    "pointer-version", "plan-version", "state-version", "receipt-version", "plan-action", "state-identity",
])
def test_corrupt_activation_authority_refuses_before_effects(sandbox, monkeypatch, corruption):
    effect = sandbox / "must-not-run"
    repo = fixture_repo(sandbox, append_script(effect))
    if corruption == "receipt-version":
        assert grip("apply", "--host", "testhost", cwd=repo).returncode == 0
    else:
        monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "hook-after-start")
        assert grip("apply", "--host", "testhost", cwd=repo).returncode != 0
    original_effect = effect.read_bytes() if effect.exists() else None
    row = outcomes(repo)[0]
    home = sandbox / ".local/share/gripsack"
    instance = home / "activation" / row["activation"]
    if corruption.startswith("pointer"):
        target = home / "activation.json"
    elif corruption.startswith("plan"):
        target = instance / "plan.json"
    elif corruption.startswith("receipt"):
        target = instance / "receipt.json"
        # Restore only the pointer, as in the archive-before-unlink crash
        # window; the receipt itself is an otherwise valid real outcome.
        (home / "activation.json").write_text(json.dumps({"version": 2, "instance": row["activation"]}))
    else:
        target = instance / "outcomes" / (row["intent"] + ".json")
    value = json.loads(target.read_text())
    if corruption.endswith("version"):
        value["version"] = 99
    elif corruption == "plan-action":
        value["intents"][0]["action"]["script"] = "touch " + shlex.quote(str(effect))
    else:
        value["intent"] = "0" * 64
    target.write_text(json.dumps(value))
    corrupted = target.read_bytes()
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER", raising=False)
    refused = grip("apply", "--host", "testhost", cwd=repo)
    assert refused.returncode != 0
    assert (effect.read_bytes() if effect.exists() else None) == original_effect
    assert target.read_bytes() == corrupted
    assert (home / "activation.json").exists()


def test_exhausted_attempt_counter_retains_evidence_without_launch(sandbox, monkeypatch):
    effect = sandbox / "must-not-run"
    repo = fixture_repo(sandbox, append_script(effect))
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "hook-after-start")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode != 0
    row = outcomes(repo)[0]
    home = sandbox / ".local/share/gripsack"
    state = home / "activation" / row["activation"] / "outcomes" / (row["intent"] + ".json")
    value = json.loads(state.read_text())
    value["state"]["attempt"] = 18446744073709551615
    state.write_text(json.dumps(value))
    before = state.read_bytes()
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    refused = grip("apply", "--host", "testhost", cwd=repo)
    assert refused.returncode != 0
    assert not effect.exists() and state.read_bytes() == before
    assert outcomes(repo)[0]["state"]["attempt"] == 18446744073709551615


def test_retained_v1_activation_replays_without_changing_identity(sandbox, monkeypatch):
    effect = sandbox / "legacy-version-effects"
    repo = fixture_repo(sandbox, append_script(effect))
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "before-adapters")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode != 0
    home = sandbox / ".local/share/gripsack"
    pointer_path = home / "activation.json"
    pointer = json.loads(pointer_path.read_text())
    instance = home / "activation" / pointer["instance"]
    # Legacy action bytes and identities are unchanged. Reconstitute exactly
    # the previous version's pending wire, not a fake process/outcome receipt.
    for path in [pointer_path, instance / "plan.json", *sorted((instance / "outcomes").glob("*.json"))]:
        document = json.loads(path.read_text())
        assert document["version"] == 2
        document["version"] = 1
        path.write_text(json.dumps(document))
    before = outcomes(repo)
    assert before[0]["state"]["kind"] == "pending"
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    resumed = grip("apply", "--host", "testhost", cwd=repo)
    assert resumed.returncode == 0, resumed.stdout + resumed.stderr
    assert effect.read_text().splitlines() == [f'{before[0]["intent"]}:1']
    assert outcomes(repo)[0]["intent"] == before[0]["intent"]
    assert json.loads((instance / "receipt.json").read_text())["version"] == 1


@pytest.mark.parametrize("plan_version,pointer_version", [(1, 2), (2, 1)])
def test_pointer_plan_version_mismatch_refuses_before_effect(
        sandbox, monkeypatch, plan_version, pointer_version):
    effect = sandbox / "must-not-run"
    repo = fixture_repo(sandbox, append_script(effect))
    monkeypatch.setenv("GRIPSACK_CRASH_AFTER", "before-adapters")
    assert grip("apply", "--host", "testhost", cwd=repo).returncode != 0
    home = sandbox / ".local/share/gripsack"
    pointer_path = home / "activation.json"
    pointer = json.loads(pointer_path.read_text())
    instance = home / "activation" / pointer["instance"]
    records = [pointer_path, instance / "plan.json",
               *sorted((instance / "outcomes").glob("*.json"))]
    # Establish a coherent legacy-action plan/outcome set in either supported
    # version. Inspection must admit it before the one-field mutation below.
    for path in records:
        document = json.loads(path.read_text())
        assert document["version"] == 2
        document["version"] = plan_version
        path.write_text(json.dumps(document))
    admitted = outcomes(repo)
    assert admitted[0]["state"]["kind"] == "pending"
    assert not effect.exists()
    pointer["version"] = pointer_version
    pointer_path.write_text(json.dumps(pointer))
    before = {path: path.read_bytes() for path in records}
    generation = (home / "current").readlink()
    monkeypatch.delenv("GRIPSACK_CRASH_AFTER")
    inspection = grip("hooks", "list", "--json", cwd=repo)
    assert inspection.returncode != 0
    refused = grip("apply", "--host", "testhost", cwd=repo)
    assert refused.returncode != 0
    assert not effect.exists()
    assert {path: path.read_bytes() for path in records} == before
    assert (home / "current").readlink() == generation
    assert (sandbox / ".owned").read_text() == "owned bytes\n"
