"""Host/controller re-admission and independent GitHub commit-status witness.

The trusted operator and its selected private installation are the authority, not
portable bundle assertions. This module never posts statuses or accesses secrets.
"""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import re
import stat
import tempfile

import release_manifest as schema

CONTEXT = "local-qualification"
REPOSITORY_ID = 1342976637
CREATOR_ID = 19335185
MAX_STATUS_PAGES = 10


def receipt_projection(receipt, original_digest):
    return {field: receipt[field] for field in (
        "candidate_id", "source_commit", "worker_image", "adapter_sha256", "gate",
        "argv", "attempt", "outcome", "returncode", "outputs")} | {
            "format": "gripsack.gate-evidence.v1", "receipt_sha256": original_digest}


def status_payload(binding):
    from release_operator import validate_binding
    validate_binding(binding)
    return {"context": CONTEXT, "state": "success",
            "description": "manifest-sha256:" + binding["manifest_sha256"],
            "target_url": binding["bundle_url"]}


def validate_status(binding, repository, statuses):
    return validate_context_status(binding, repository, statuses, status_payload(binding))


def validate_context_status(binding, repository, statuses, expected):
    """Validate the newest fixed-context record, never an older convenient success."""
    from release_operator import validate_binding
    validate_binding(binding)
    schema.require(set(expected) == {"context", "state", "description", "target_url"}
                   and expected["context"] in (CONTEXT, "release-consumer-verification")
                   and expected["state"] == "success", "unsupported witness policy")
    schema.require(isinstance(repository, dict) and type(repository.get("id")) is int
                   and repository["id"] == REPOSITORY_ID
                   and repository.get("full_name") == schema.REPOSITORY,
                   "foreign witness repository identity")
    schema.require(isinstance(statuses, list)
                   and all(isinstance(item, dict) and isinstance(item.get("context"), str)
                           for item in statuses), "invalid witness status response")
    matching = [item for item in statuses if item["context"] == expected["context"]]
    schema.require(matching, "missing local-qualification witness")
    latest = matching[0]
    identifier = latest.get("id")
    schema.require(type(identifier) is int and identifier > 0
                   and all(type(item.get("id")) is int and item["id"] <= identifier for item in matching),
                   "unordered or stale witness status response")
    schema.require(latest.get("url") == f"https://api.github.com/repos/{schema.REPOSITORY}/statuses/{binding['source_commit']}",
                   "witness belongs to another source")
    creator = latest.get("creator")
    schema.require(isinstance(creator, dict) and type(creator.get("id")) is int and creator["id"] == CREATOR_ID,
                   "foreign local-qualification witness creator")
    schema.require(all(latest.get(key) == value for key, value in expected.items()),
                   "latest local-qualification witness is pending, failed, stale or mismatched")
    return latest


def fetch_status(binding, api):
    return fetch_context_status(binding, status_payload(binding), api)


def fetch_context_status(binding, expected, api):
    """Read independently from the fixed repository's bounded status history."""
    from release_operator import validate_binding
    validate_binding(binding)
    repository = api(f"repos/{schema.REPOSITORY}")
    statuses = []
    for page in range(1, MAX_STATUS_PAGES + 1):
        rows = api(f"repos/{schema.REPOSITORY}/commits/{binding['source_commit']}/statuses?per_page=100&page={page}")
        schema.require(isinstance(rows, list) and len(rows) <= 100, "invalid witness status page")
        statuses.extend(rows)
        if any(isinstance(row, dict) and row.get("context") == expected["context"] for row in rows) or len(rows) < 100:
            return validate_context_status(binding, repository, statuses, expected)
    raise ValueError("local-qualification witness search exceeds bounded history")


def private_installation(state):
    from ci_infra.bootstrap import configuration
    state = Path(state).absolute()
    # The selector is host-owned installation metadata, never a bundle path or
    # caller-supplied candidate/evidence hierarchy. No private credentials read.
    for path, directory in ((state, True), (state / "config.json", False)):
        info = path.lstat()
        schema.require(not path.is_symlink() and info.st_uid == os.getuid() and not info.st_mode & 0o077
                       and (stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode)),
                       "selected CI installation metadata must be private and operator-owned")
    settings = configuration(state)
    schema.require(isinstance(settings.get("installation"), str)
                   and re.fullmatch(r"[0-9a-f]{16}", settings["installation"]), "invalid selected installation identity")
    schema.require(isinstance(settings.get("image"), str) and re.fullmatch(r"sha256:[0-9a-f]{64}", settings["image"])
                   and isinstance(settings.get("adapter_sha256"), str)
                   and schema.HEX64.fullmatch(settings["adapter_sha256"]), "invalid selected runtime identity")
    return state, settings


def admit_private(root, source, state, manifest):
    """Re-admit the selected controller archive; no alternate evidence inputs."""
    from ci_infra import records
    state, settings = private_installation(state)
    identifier = manifest["candidate_id"]
    frozen = state / "inbox" / identifier
    candidate = records.load_candidate(frozen / "candidate.json")
    schema.require(candidate["candidate_id"] == identifier
                   and candidate == schema.load_json(root / "evidence/candidate.json"),
                   "private selected candidate differs from nomination")
    inputs = candidate["inputs"]
    schema.require(inputs["worker_image"] == settings["image"]
                   and inputs["adapter_sha256"] == settings["adapter_sha256"], "candidate differs from selected runtime")
    schema.require(inputs["repository"] == f"https://github.com/{schema.REPOSITORY}.git"
                   and {key: inputs[key] for key in ("commit", "tree")} == manifest["source"]
                   == schema.clean_source(source), "private candidate source differs")
    schema.require(inputs["profile"] == schema.load_json(Path(source) / ".ci/pilot.json")
                   and hashlib.sha256(schema.canonical(inputs["profile"])).hexdigest() == manifest["recipe_sha256"],
                   "private candidate profile differs")
    schema.require(schema.digest(frozen / "source.bundle") == inputs["bundle_sha256"], "private source archive differs")
    evidence = state / "archive" / identifier / "evidence"
    qualification = records.admit(candidate, evidence)
    schema.require(qualification == schema.load_json(root / "evidence/qualification.json"),
                   "portable qualification differs from private admission")
    for gate in inputs["profile"]["gates"]:
        attempt = records.passed_attempt(candidate, evidence, gate["id"])
        schema.require(attempt is not None, "missing privately admitted gate")
        receipt_path = attempt / "receipt.json"
        with schema.regular(receipt_path).open("rb") as stream:
            data = stream.read(schema.MAX_JSON_BYTES + 1)
        receipt = schema.decode_json(data)
        original_digest = hashlib.sha256(data).hexdigest()
        projection = receipt_projection(receipt, original_digest)
        schema.require(projection == schema.load_json(root / f"evidence/gates/{gate['id']}.json"),
                       "portable projection differs from original private receipt")
        attribution = next(row for row in qualification["gates"] if row["gate"] == gate["id"])
        schema.require(attribution["receipt_sha256"] == original_digest
                       and attribution["attempt"] == receipt["attempt"], "private receipt changed during admission")
    schema.require(inputs["profile"].get("external_inputs", {}).get("authority") == schema.POLICY,
                   "private candidate lacks standing local qualification authority")
    return qualification


def admit_local(source, state, archive, bundle_url):
    """Return {binding, status} only after actual selected private re-admission.

    `state` is the operator-selected ci_infra installation root, not transported
    evidence. The caller authenticates its GitHub identity and posts/readbacks
    the returned status; this function performs no API writes.
    """
    from release_operator import host_only, make_binding
    host_only()
    archive = Path(archive)
    bundle_digest = schema.digest(archive)
    with tempfile.TemporaryDirectory(prefix="gripsack-readmit-") as directory:
        root = Path(directory) / "bundle"
        schema.extract_archive(archive, root)
        manifest = schema.validate_bundle(root, source=source)
        path = root / f"gripsack-release-{manifest['compatible']['core']}.json"
        binding = make_binding(path, bundle_url, bundle_digest)
        admit_private(root, source, state, manifest)
        schema.require(schema.digest(archive) == bundle_digest, "transport changed during private admission")
        return {"binding": binding, "status": status_payload(binding)}
