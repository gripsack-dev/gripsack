"""Real standalone installer with offline HTTP and an explicit verifier double.

These tests prove admission/control flow and preservation, NOT cryptographic
success. Release promotion additionally requires real-gh signed-candidate smoke.
"""
import os
from pathlib import Path
import subprocess
import sys

import pytest
from release_fixture import REJECTED_IR_CONTRACTS, REPO, Releases, archive


@pytest.fixture
def installer(tmp_path):
    server = Releases()
    server.serve("9.9.9", archive("9.9.9"))
    destination = tmp_path / "bin/grip"
    destination.parent.mkdir()
    destination.write_bytes(b"old executable\n")
    environment = server.environment(tmp_path)
    environment["GRIPSACK_BIN"] = str(destination.parent)

    def run(**extra):
        return subprocess.run(["sh", str(REPO / "install.sh")], env=environment | extra,
                              capture_output=True, text=True, timeout=60)
    try:
        yield server, destination, environment, run
    finally:
        server.release_tarball()
        server._server.shutdown()
        server._server.server_close()


def test_valid_candidate_installs_verified_payload_without_executing_it(installer):
    server, destination, _, run = installer
    payload = b"#!/bin/sh\nexit 93\n"
    server.serve("9.9.9", archive("9.9.9", payload))
    result = run()
    assert result.returncode == 0, result.stderr
    assert destination.read_bytes() == payload
    assert os.access(destination, os.X_OK)


@pytest.mark.parametrize("mutation", ["wrong-repo", "wrong-workflow", "wrong-ref", "wrong-source", "wrong-policy",
                                      "incompatible-helper", "incompatible-ir", "incompatible-sdk", "missing-gate",
                                      "missing-signature", "invalid-signature", "tampered-asset", "missing-asset", "extra-asset",
                                      "corrupt-extraction"])
def test_rejected_candidate_preserves_old_installation(installer, mutation):
    server, destination, _, run = installer
    assets = server.releases["9.9.9"]["assets"]
    original = destination.read_bytes()
    changes = {
        "wrong-repo": lambda m: m.update(repository="attacker/gripsack"),
        "wrong-workflow": lambda m: m["signer"].update(workflow=".github/workflows/other.yml"),
        "wrong-ref": lambda m: m["signer"].update(ref="refs/heads/main"),
        "wrong-source": lambda m: m["source"].update(commit="0" * 40),
        "wrong-policy": lambda m: m.update(policy="checksum-only"),
        "incompatible-helper": lambda m: m["compatible"].update(conda="0.0.1"),
        "incompatible-ir": lambda m: m["compatible"]["ir"].update(writer=99),
        "incompatible-sdk": lambda m: m["packages"][0].update(version="0.0.1"),
        "missing-gate": lambda m: m["gates"].pop(),
    }
    if mutation in changes:
        server.mutate_manifest("9.9.9", changes[mutation])
    elif mutation == "missing-signature":
        del assets["gripsack-release-9.9.9.json.sigstore.json"]
    elif mutation == "invalid-signature":
        assets["gripsack-release-9.9.9.json.sigstore.json"] = b"not a signature"
    elif mutation == "extra-asset":
        assets["unqualified"] = b"extra"
    elif mutation == "missing-asset":
        del assets["grip-buildkit-bridge-9.9.9-x86_64-unknown-linux-musl"]
    elif mutation == "tampered-asset":
        assets["grip-buildkit-bridge-9.9.9-x86_64-unknown-linux-musl"] = b"BRIDGE"
    elif mutation == "corrupt-extraction":
        server.serve("9.9.9", b"not a tar archive")
    result = run()
    assert result.returncode != 0, result.stdout
    assert destination.read_bytes() == original


@pytest.mark.parametrize("contract", [contract for _, contract in REJECTED_IR_CONTRACTS],
                         ids=[name for name, _ in REJECTED_IR_CONTRACTS])
def test_noncurrent_ir_contract_rejects_before_payload_download(installer, contract):
    server, destination, _, run = installer
    server.mutate_manifest("9.9.9", lambda m: m["compatible"].update(ir=contract))
    result = run()
    assert result.returncode != 0
    assert "unsupported IR compatibility contract" in result.stderr
    assert destination.read_bytes() == b"old executable\n"
    assert server.count(".tar.gz") == 0


@pytest.mark.parametrize("reason", ["wrong-signer", "wrong-repository-certificate", "wrong-tag-certificate", "wrong-source-certificate", "bad-signature", "missing-signature"])
def test_verifier_refusal_is_never_a_checksum_fallback(installer, reason):
    _, destination, _, run = installer
    result = run(FIXTURE_SIGNATURE_FAILURE=reason)
    assert result.returncode != 0
    assert destination.read_bytes() == b"old executable\n"


def test_verified_but_wrong_custom_predicate_is_rejected(installer):
    _, destination, _, run = installer
    assert run(FIXTURE_WRONG_PREDICATE="1").returncode != 0
    assert destination.read_bytes() == b"old executable\n"


def test_missing_verifier_fails_before_any_network(installer):
    server, destination, environment, run = installer
    gh = Path(environment["PATH"].split(os.pathsep)[0]) / "gh"
    gh.write_text("#!/bin/sh\necho 'gh version 2.0.0'\n")
    result = run()
    assert result.returncode != 0
    assert not server.requests
    assert destination.read_bytes() == b"old executable\n"


@pytest.mark.parametrize("system,machine", [("Darwin", "x86_64"), ("Linux", "aarch64")])
def test_unsupported_platform_fails_before_network(installer, system, machine):
    server, destination, environment, run = installer
    uname = Path(environment["PATH"].split(os.pathsep)[0]) / "uname"
    uname.write_text(f'#!/bin/sh\nif [ "$1" = -s ]; then echo {system}; else echo {machine}; fi\n')
    uname.chmod(0o755)
    assert run().returncode != 0
    assert not server.requests
    assert destination.read_bytes() == b"old executable\n"


def test_absent_verifier_is_a_hard_error(monkeypatch):
    sys.path.insert(0, str(REPO / "scripts"))
    import release_consumer
    monkeypatch.setattr(release_consumer.shutil, "which", lambda _: None)
    with pytest.raises(ValueError, match="no unsigned fallback"):
        release_consumer.verifier()


def test_interrupted_atomic_replace_preserves_old_bytes(tmp_path, monkeypatch):
    sys.path.insert(0, str(REPO / "scripts"))
    import release_consumer
    source = tmp_path / "candidate"
    source.write_bytes(b"new executable")
    destination = tmp_path / "grip"
    destination.write_bytes(b"old executable")
    def interrupted(*args, **kwargs):
        raise InterruptedError("injected before rename commit")
    monkeypatch.setattr(release_consumer.os, "replace", interrupted)
    with pytest.raises(InterruptedError):
        release_consumer.publish(source, destination)
    assert destination.read_bytes() == b"old executable"
    assert not list(tmp_path.glob(".grip-install-*"))


def test_postcommit_durability_error_never_rolls_back(tmp_path, monkeypatch):
    sys.path.insert(0, str(REPO / "scripts"))
    import release_consumer
    source = tmp_path / "candidate"
    source.write_bytes(b"new executable")
    destination = tmp_path / "grip"
    destination.write_bytes(b"old executable")
    real = release_consumer.os.fsync
    def fsync(fd):
        import stat
        if stat.S_ISDIR(release_consumer.os.fstat(fd).st_mode):
            raise OSError("injected directory durability failure")
        real(fd)
    monkeypatch.setattr(release_consumer.os, "fsync", fsync)
    with pytest.raises(release_consumer.PublicationCommittedError):
        release_consumer.publish(source, destination)
    assert destination.read_bytes() == b"new executable"
