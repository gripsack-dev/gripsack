#!/usr/bin/env python3
"""Read-only admission of retained host release observations, not caller assertions.

The trusted host operator is the observation root. These records are not a
cryptographic proof against that operator forging its own files or process logs.
"""
from __future__ import annotations

import hashlib
from pathlib import Path
import tempfile

import release_consumer as consumer
import release_manifest as schema
import release_publish as publish
from local_artifact_consumers import EXTERNAL

FORMAT = "gripsack.release-completion.v2"
SURFACE_FORMAT = "gripsack.release-observation.v1"
SITE_URL = "https://gripsack.dev/release.json"
MAX_LOG = 16 * 1024 * 1024
ENV_KEYS = frozenset({"PATH", "LANG", "LC_ALL", "TZ", "HOME", "TMPDIR", "GRIPSACK_HOME",
                      "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "CARGO_HOME",
                      "NPM_CONFIG_USERCONFIG", "NPM_CONFIG_GLOBALCONFIG", "NPM_CONFIG_CACHE",
                      "GIT_CONFIG_NOSYSTEM", "GIT_CONFIG_GLOBAL", "GIT_OPTIONAL_LOCKS",
                      "PYTHONDONTWRITEBYTECODE", "PYTEST_DISABLE_PLUGIN_AUTOLOAD"})
require = schema.require
CORE_RECIPES = ("install.sh", "scripts/local_artifact_consumers.py",
                "examples/workspaces/03-source-built/gripsack.ts",
                "examples/workspaces/03-source-built/source/greeter.c")


def object_digest(kind, data):
    return hashlib.sha1(f"{kind} {len(data)}\0".encode() + data).hexdigest()


def tree_entries(data):
    entries = {}
    while data:
        header, separator, rest = data.partition(b"\0")
        require(separator and len(rest) >= 20, "invalid retained Git tree")
        mode, name = header.decode().split(" ", 1)
        require(name not in entries and "/" not in name and name not in (".", ".."), "invalid Git tree name")
        entries[name] = (mode, rest[:20].hex())
        data = rest[20:]
    return entries


def source_proof(root, proof, expected_commit, files):
    """Admit byte identities through Git's actual commit/tree/blob object chain."""
    schema.keys(proof, {"commit", "trees", "files"})
    commit = resolve(root, proof["commit"], schema.MAX_JSON_BYTES).read_bytes()
    require(object_digest("commit", commit) == expected_commit, "recipe source commit differs")
    first = commit.split(b"\n", 1)[0].decode()
    require(first.startswith("tree ") and schema.HEX40.fullmatch(first[5:]), "invalid source commit tree")
    tree = first[5:]
    decoded = {}
    for digest, ref in proof["trees"].items():
        raw = resolve(root, ref, schema.MAX_JSON_BYTES).read_bytes()
        require(object_digest("tree", raw) == digest, "source tree object differs")
        decoded[digest] = tree_entries(raw)
    if files is None:
        files = []
        def visit(identifier, prefix="", depth=0):
            require(depth <= 32 and len(files) <= schema.MAX_FILES, "source proof exceeds bound")
            for name, (mode, child) in decoded[identifier].items():
                relative = prefix + name
                if mode == "40000":
                    visit(child, relative + "/", depth + 1)
                else:
                    require(mode in {"100644", "100755"}, "external recipe has nonregular entries")
                    files.append(relative)
        visit(tree)
    require(set(proof["files"]) == set(files), "missing or extra frozen recipe files")
    result = {}
    for relative in files:
        parts = schema.safe_path(relative).split("/")
        current = tree
        for part in parts[:-1]:
            mode, current = decoded[current][part]
            require(mode == "40000", "recipe parent is not a Git directory")
        mode, expected = decoded[current][parts[-1]]
        require(mode in {"100644", "100755"}, "recipe is not a regular source blob")
        file = resolve(root, proof["files"][relative], schema.MAX_JSON_BYTES)
        require(object_digest("blob", file.read_bytes()) == expected, "recipe bytes differ from frozen source")
        result[relative] = file
    return tree, result


def required_surfaces(manifest):
    """Applicability comes from the admitted contract, never caller waivers."""
    schema.validate_manifest(manifest)
    result = {"signed_manifest", "downloaded_assets", "installer", "site"}
    if manifest["packages"]:
        result.add("registries")
    if any(p["registry"] == "npm" for p in manifest["packages"]):
        result.add("sdk")
    if {a["role"] for a in manifest["artifacts"]} & {"conda", "bridge"}:
        result.add("default_helpers")
    return result


def reference(root, path):
    path = schema.regular(path)
    require(path.is_relative_to(root), "evidence outside retained directory")
    return {"path": str(path.relative_to(root)), "size_bytes": path.stat().st_size,
            "sha256": schema.digest(path)}


def resolve(root, item, maximum=schema.MAX_FILE_BYTES):
    schema.keys(item, {"path", "size_bytes", "sha256"})
    schema.safe_path(item["path"])
    require(type(item["size_bytes"]) is int and 0 <= item["size_bytes"] <= maximum,
            "evidence byte size outside bound")
    require(isinstance(item["sha256"], str) and schema.HEX64.fullmatch(item["sha256"]),
            "invalid evidence digest")
    path = schema.regular(root / item["path"])
    require(path.stat().st_size == item["size_bytes"] and schema.digest(path) == item["sha256"],
            "referenced evidence bytes differ")
    return path


def identity(record, run_id, release_id):
    require(type(run_id) is int and run_id > 0 and type(release_id) is int and release_id > 0,
            "invalid workflow/release identity")
    return {"request_id": record["request_id"], "binding": record["binding"],
            "run_id": run_id, "release_id": release_id}


def bound_manifest(manifest, digest, record):
    binding = record["binding"]
    require(digest == binding["manifest_sha256"]
            and manifest["candidate_id"] == binding["candidate_id"]
            and manifest["source"]["commit"] == binding["source_commit"]
            and manifest["signer"]["ref"] == "refs/tags/" + binding["tag"],
            "manifest differs from durable candidate/source binding")


def command(root, item):
    schema.keys(item, {"step", "argv", "cwd", "environment", "returncode", "termination",
                       "duration_seconds", "stdout", "stderr", "stdin"})
    require(isinstance(item["argv"], list) and item["argv"]
            and all(isinstance(a, str) and a and len(a) <= 8192 for a in item["argv"]),
            "invalid executed argv")
    require(item["termination"] == "exited" and type(item["returncode"]) is int
            and item["returncode"] == 0, "consumer process did not execute successfully")
    require(type(item["duration_seconds"]) in (int, float)
            and 0 <= item["duration_seconds"] <= 3605, "invalid command duration")
    environment = item["environment"]
    require(isinstance(environment, dict) and set(environment) <= ENV_KEYS,
            "candidate command exposes ambient credentials or overrides")
    for name in ("HOME", "TMPDIR", "GRIPSACK_HOME", "CARGO_HOME", "XDG_CONFIG_HOME",
                 "XDG_CACHE_HOME", "XDG_DATA_HOME", "NPM_CONFIG_CACHE"):
        value = environment.get(name)
        require(isinstance(value, str) and Path(value).is_relative_to(root)
                and ".." not in Path(value).parts, "candidate home is not privately owned")
    require(environment.get("GIT_CONFIG_GLOBAL") == "/dev/null"
            and environment.get("GIT_CONFIG_NOSYSTEM") == "1", "candidate command reads operator configuration")
    for key in ("NPM_CONFIG_USERCONFIG", "NPM_CONFIG_GLOBALCONFIG"):
        config = Path(environment.get(key, "/nonexistent"))
        require(config.is_relative_to(root) and schema.regular(config).stat().st_size == 0,
                "candidate command reads nonempty or foreign npm configuration")
    require(Path(item["cwd"]).is_relative_to(root) and ".." not in Path(item["cwd"]).parts,
            "consumer working directory is not owned")
    out = resolve(root, item["stdout"], MAX_LOG).read_bytes()
    resolve(root, item["stderr"], MAX_LOG)
    if item["stdin"] is not None:
        resolve(root, item["stdin"], MAX_LOG)
    return out.decode("utf-8", errors="replace")


def steps(root, surface, wanted):
    entries = surface["commands"]
    require(isinstance(entries, list) and len(entries) <= 256, "invalid command inventory")
    names = [entry.get("step") for entry in entries]
    require(len(names) == len(set(names)) and set(names) == set(wanted),
            "missing, duplicate or extra executed consumer steps")
    return {entry["step"]: (entry, command(root, entry)) for entry in entries}


def argv(entry, prefix, suffix=()):
    actual = entry["argv"]
    require(actual == list(map(str, (*prefix, *suffix))),
            "recorded command does not execute required recipe")


def approval(commands, prefix, executable, repo):
    inspected, output = commands[prefix + "_inspect"]
    argv(inspected, [str(executable), "trust", "inspect", str(repo), "--json"])
    value = schema.decode_json(output.encode())
    require(value.get("repository") == str(repo), "approval repository differs")
    for key in ("bundle_digest", "policy_digest"):
        require(isinstance(value.get(key), str) and value[key], "missing inspected approval identity")
    approved, _ = commands[prefix + "_approve"]
    argv(approved, [str(executable), "trust", "add", str(repo), "--bundle",
                    value["bundle_digest"], "--policy", value["policy_digest"]])


def check_output(output, legacy=False):
    value = schema.decode_json(output.encode())
    require(value.get("ok") is True and value.get("host", {}).get("arch") == "x86_64"
            and value.get("host", {}).get("os") == "linux"
            and bool(value.get("modules" if legacy else "outputs")), "consumer did not evaluate real source outputs")


class ReplayRegistry(publish.Registry):
    """Reapply the production registry validator to the retained HTTP bytes."""
    def __init__(self, root, responses):
        super().__init__(environ={}, sleep=lambda _: None)
        self.root, self.responses, self.index = root, responses, 0

    def request(self, url, *, method="GET", data=None, token=None,
                limit=publish.MAX_METADATA, content_type="application/json"):
        require(method == "GET" and data is None and token is None, "non-read-only registry observation")
        require(self.index < len(self.responses), "registry response not retained")
        entry = self.responses[self.index]
        self.index += 1
        schema.keys(entry, {"url", "body"})
        require(entry["url"] == url, "registry response URL mismatch")
        return resolve(self.root, entry["body"], limit).read_bytes()


def validate_site(value, manifest, ident):
    expected = {"format": "gripsack.site-release.v1", "repository": manifest["repository"],
                "source": manifest["source"], "candidate_id": manifest["candidate_id"],
                "manifest_sha256": ident["binding"]["manifest_sha256"],
                "tag": ident["binding"]["tag"], "run_id": ident["run_id"],
                "release_id": ident["release_id"], "version": manifest["compatible"]["core"]}
    require(value == expected, "live site is stale or not bound to this release")


def verify_completion(path, expected_digest, record, run):
    """Admit retained content and fresh signatures, not historical log authority.

    The observer must additionally authenticate this exact receipt digest using
    the independent trusted-host completion witness. Caller-authored process
    logs alone are never authority that the historical commands actually ran.
    """
    path = schema.regular(path)
    root = path.parent
    require(schema.digest(path) == expected_digest, "completion receipt digest mismatch")
    receipt = schema.load_json(path)
    schema.keys(receipt, {"format", "identity", "status", "manifest", "surfaces"})
    require(receipt["format"] == FORMAT and receipt["status"] == "complete",
            "release consumers are pending or unknown")
    ident = receipt["identity"]
    require(ident == identity(record, run["id"], ident["release_id"]), "completion release identity mismatch")
    require(run.get("status") == "completed" and run.get("conclusion") == "success",
            "completion cannot override unfinished or unsuccessful workflow")
    manifest_path = resolve(root, receipt["manifest"], schema.MAX_JSON_BYTES)
    manifest = schema.validate_manifest(schema.load_json(manifest_path))
    bound_manifest(manifest, receipt["manifest"]["sha256"], record)
    require(set(receipt["surfaces"]) == required_surfaces(manifest), "missing or extra required surfaces")
    surfaces = {}
    for name, ref in receipt["surfaces"].items():
        surface = schema.load_json(resolve(root, ref, schema.MAX_JSON_BYTES))
        schema.keys(surface, {"format", "surface", "identity", "status", "commands", "data", "detail"})
        require(surface["format"] == SURFACE_FORMAT and surface["surface"] == name
                and surface["identity"] == ident and surface["status"] == "verified"
                and surface["detail"] is None, "surface identity/status is not verified")
        surfaces[name] = surface
    version = manifest["compatible"]["core"]
    signed = surfaces["signed_manifest"]
    schema.keys(signed["data"], {"bundle", "release", "tag_source", "source_proof", "transport", "workflow_run"})
    from release_operator import validate_run
    observed_run = validate_run(schema.load_json(resolve(root, signed["data"]["workflow_run"])), record)
    require(observed_run["id"] == run["id"] and observed_run.get("status") == "completed"
            and observed_run.get("conclusion") == "success", "retained workflow observation differs")
    tree, recipes = source_proof(root, signed["data"]["source_proof"], manifest["source"]["commit"], CORE_RECIPES)
    require(tree == manifest["source"]["tree"], "recipe source tree differs")
    release = schema.load_json(resolve(root, signed["data"]["release"], schema.MAX_JSON_BYTES))
    require(release.get("id") == ident["release_id"] and release.get("tag_name") == "core-v" + version
            and release.get("draft") is False and release.get("prerelease") is False,
            "release observation identity differs")
    require(schema.load_json(resolve(root, signed["data"]["tag_source"])) == manifest["source"],
            "release tag source differs")
    signature = resolve(root, signed["data"]["bundle"], consumer.MAX_JSON)
    transport = schema.load_json(resolve(root, signed["data"]["transport"], schema.MAX_JSON_BYTES))
    require(transport.get("identity") == ident and transport.get("surface") == "transport_setup",
            "transport observation release identity differs")
    transport_steps = steps(root, transport, {"manifest", "signature", "verifier_version"})
    require(transport_steps["manifest"][0]["argv"] == download_argv(public_asset_url(version, manifest_path.name),
                                                                  manifest_path, schema.MAX_JSON_BYTES),
            "manifest transport was not observed")
    require(transport_steps["signature"][0]["argv"] == download_argv(public_asset_url(version, signature.name),
                                                                   signature, consumer.MAX_JSON),
            "signature transport was not observed")
    argv(transport_steps["verifier_version"][0], ["gh", "--version"])
    signed_steps = steps(root, signed, {"attestation_bundle", "attestation_repository"})
    recorded = iter(signed_steps[name] for name in ("attestation_bundle", "attestation_repository"))
    def verify_command(args):
        entry, _ = next(recorded)
        require(entry["argv"] == list(map(str, args)), "signature verifier command differs")
        # Never treat retained gh JSON as signature authority. Re-execute the
        # independently provisioned verifier on the actual manifest and bundle.
        return consumer.command(args)
    consumer.verifier()
    consumer.verify_attestation(manifest_path, manifest, signature, command_runner=verify_command)

    assets = surfaces["downloaded_assets"]
    schema.keys(assets["data"], {"artifacts", "core"})
    observed = assets["data"]["artifacts"]
    require(set(observed) == {a["path"] for a in manifest["artifacts"]}, "downloaded artifact inventory differs")
    fetched = steps(root, assets, {a["path"] for a in manifest["artifacts"]})
    for item in manifest["artifacts"]:
        downloaded = resolve(root, observed[item["path"]])
        require(schema.digest(downloaded) == item["sha256"] and downloaded.stat().st_size == item["size_bytes"],
                "downloaded artifact bytes differ from signed manifest")
        entry, _ = fetched[item["path"]]
        require(entry["argv"] == download_argv(public_asset_url(version, downloaded.name), downloaded, item["size_bytes"]),
                "asset was not fetched from the release")
    core = resolve(root, assets["data"]["core"])
    core_item = next(a for a in manifest["artifacts"] if a["role"] == "core")
    with tempfile.TemporaryDirectory(prefix="gripsack-evidence-core-") as temporary:
        extracted = Path(temporary) / "grip"
        consumer.extract_core(resolve(root, observed[core_item["path"]]), version, extracted)
        require(schema.digest(core) == schema.digest(extracted), "installed core identity is not archive payload")

    registry = surfaces["registries"]
    require(not registry["commands"], "unexpected registry shell execution")
    schema.keys(registry["data"], {"responses", "packages"})
    remote = ReplayRegistry(root, registry["data"]["responses"])
    require(set(registry["data"]["packages"]) == {p["path"] for p in manifest["packages"]},
            "registry package inventory differs")
    for package in manifest["packages"]:
        require(remote.remote_digest(package) == package["sha256"], "registry payload not verified")
        payload = resolve(root, registry["data"]["packages"][package["path"]])
        require(schema.digest(payload) == package["sha256"] and payload.stat().st_size == package["size_bytes"],
                "retained registry bytes differ")
    require(remote.index == len(remote.responses), "extra registry responses")

    install = surfaces["installer"]
    schema.keys(install["data"], {"installer", "installed", "before_update", "after_update", "previous_version", "mode"})
    installed = resolve(root, install["data"]["installed"])
    before = resolve(root, install["data"]["before_update"])
    after = resolve(root, install["data"]["after_update"])
    installer = resolve(root, install["data"]["installer"])
    require(schema.digest(installer) == schema.digest(recipes["install.sh"]), "installer is not frozen source")
    require(schema.digest(installed) == schema.digest(core) == schema.digest(after), "wrong installed/updated bytes")
    require((before.stat().st_dev, before.stat().st_ino) != (after.stat().st_dev, after.stat().st_ino),
            "updater was a no-op; executable inode was not replaced")
    commands = steps(root, install, {"install", "installed_version", "update", "updated_version"})
    argv(commands["install"][0], ["sh", str(installer), "--version", version])
    for name in ("installed_version", "updated_version"):
        require(commands[name][1].strip() == f"grip {version}", "installed version process output differs")
        argv(commands[name][0], [str(installed if name == "installed_version" else after), "--version"])
    mode = install["data"]["mode"]
    require(mode in {"upgrade", "reinstall"}, "unrecognized updater execution")
    if mode == "reinstall":
        require(install["data"]["previous_version"] == version and schema.digest(before) == schema.digest(after),
                "reinstall baseline differs from verified candidate")
        argv(commands["update"][0], [str(after), "self-update", "--reinstall", "--version", version])
        require(f"reinstalled {version}" in commands["update"][1], "embedded reinstall did not perform replacement")
    else:
        previous = consumer.stable_version("core-v" + install["data"]["previous_version"])
        require(previous is not None and previous < consumer.stable_version("core-v" + version)
                and schema.digest(before) != schema.digest(after), "updater upgrade baseline is not earlier")
        argv(commands["update"][0], [str(after), "self-update"])
        require(f"updated {install['data']['previous_version']} → {version}" in commands["update"][1],
                "embedded updater did not perform replacement")

    helpers = surfaces["default_helpers"]
    schema.keys(helpers["data"], {"installed", "conda_lock", "conda_output", "bridge_output"})
    helper_commands = steps(root, helpers, {"conda_inspect", "conda_approve", "conda_check", "conda_build", "conda_run",
                                          "bridge_inspect", "bridge_approve", "bridge_lock", "bridge_build", "bridge_run", "bridge_stop"})
    require(set(helpers["data"]["installed"]) == {"conda", "bridge"}, "absent default helper provisioning")
    for role, ref in helpers["data"]["installed"].items():
        helper = resolve(root, ref)
        require(schema.digest(helper) == manifest["compatible"]["helper_pins"][role], "wrong default helper bytes")
        namespace, filename = ("conda-helper", "gripsack-conda") if role == "conda" else ("buildkit-bridge", "bridge")
        require(helper == root / "home/.gs/tools" / f"{namespace}-{version}" / filename,
                "helper not in default provisioned cache")
    resolve(root, helpers["data"]["conda_lock"])
    conda_repo, bridge_repo = root / "conda-consumer", root / "bridge-consumer"
    approval(helper_commands, "conda", installed, conda_repo)
    approval(helper_commands, "bridge", installed, bridge_repo)
    argv(helper_commands["conda_check"][0], [str(installed), "check", "--json"])
    check_output(helper_commands["conda_check"][1])
    argv(helper_commands["bridge_lock"][0], [str(installed), "update"])
    for relative in ("gripsack.ts", "source/greeter.c"):
        require(schema.digest(schema.regular(bridge_repo / relative))
                == schema.digest(recipes["examples/workspaces/03-source-built/" + relative]),
                "default bridge consumer is not frozen source")
    for name, suffix in (("conda_build", ["build", "ripgrep", "--json"]),
                         ("conda_run", ["run", "--env", "search", "--", "rg", "^needle$"]),
                         ("bridge_build", ["build", "greeter", "--json"]),
                         ("bridge_run", ["task", "run", "smoke"]), ("bridge_stop", ["builder", "stop"])):
        argv(helper_commands[name][0], [str(installed)], suffix)
    require(helper_commands["conda_run"][1].strip() == "needle", "default helper consumer did not run real package")
    for name, key, expected_name in (("conda_build", "conda_output", "ripgrep"), ("bridge_build", "bridge_output", "greeter")):
        build = schema.decode_json(helper_commands[name][1].encode())
        require([(o.get("name"), o.get("kind")) for o in build.get("outputs", [])] == [(expected_name, "package")],
                "helper build did not produce requested package")
        output = resolve(root, helpers["data"][key])
        require(any(output.is_relative_to(Path(o["path"])) for o in build["outputs"]), "package output not retained")
    require(helper_commands["bridge_run"][1].strip() == "hello from greeter", "bridge consumer did not execute built program")

    sdk = surfaces["sdk"]
    schema.keys(sdk["data"], {"tarball", "installed_files", "package_root", "recipe", "source_proof"})
    npm = next(p for p in manifest["packages"] if p["registry"] == "npm")
    tarball = resolve(root, sdk["data"]["tarball"])
    require(schema.digest(tarball) == npm["sha256"], "SDK is not exact published tarball")
    metadata = publish.npm_metadata(tarball)
    require(metadata.get("name") == npm["name"] and metadata.get("version") == npm["version"],
            "packed SDK metadata differs from compatible release")
    sdk_steps = steps(root, sdk, {"npm_install", "sdk_inspect", "sdk_approve", "sdk_check"})
    argv(sdk_steps["npm_install"][0], ["npm", "install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund",
                                      "--package-lock=false", "--save-exact", str(tarball)])
    argv(sdk_steps["sdk_check"][0], [str(installed), "check", "--host", "laptop", "--json"])
    check_output(sdk_steps["sdk_check"][1], legacy=True)
    approval(sdk_steps, "sdk", installed, root / "sdk-consumer")
    package_root = root / schema.safe_path(sdk["data"]["package_root"])
    with tempfile.TemporaryDirectory(prefix="gripsack-evidence-sdk-") as temporary:
        schema.extract_archive(tarball, Path(temporary) / "sdk", maximum=publish.MAX_EXPANDED)
        expected = {str(p.relative_to(Path(temporary) / "sdk/package")): schema.digest(p)
                    for p in (Path(temporary) / "sdk/package").rglob("*") if p.is_file()}
    require(set(expected) == set(sdk["data"]["installed_files"]), "packed SDK file inventory differs")
    for relative, ref in sdk["data"]["installed_files"].items():
        file = resolve(root, ref)
        require(file == package_root / schema.safe_path(relative) and schema.digest(file) == expected[relative],
                "installed SDK bytes differ from packed package")
    _, external = source_proof(root, sdk["data"]["source_proof"], EXTERNAL["example"][1], None)
    require(schema.digest(resolve(root, sdk["data"]["recipe"])) == schema.digest(external["hosts/laptop.ts"]),
            "SDK recipe is not qualified frozen representative consumer")
    # npm may update package.json; every executable source and frozen lock is
    # still compared with the qualified external revision, not an ad-hoc echo.
    for relative, original in external.items():
        if relative not in {"package.json", "package-lock.json"}:
            require(schema.digest(schema.regular(root / "sdk-consumer" / relative)) == schema.digest(original),
                    "representative consumer source differs from qualified revision")
    for relative in ("gripsack.ts", "gripsack.lock"):
        original = external["workspaces/conda/" + relative]
        require(schema.digest(schema.regular(conda_repo / relative)) == schema.digest(original),
                "default Conda consumer is not frozen source")

    site = surfaces["site"]
    schema.keys(site["data"], {"url", "binding", "page"})
    require(site["data"]["url"] == SITE_URL, "site is not canonical live endpoint")
    site_steps = steps(root, site, {"site_binding", "site_page"})
    binding = resolve(root, site["data"]["binding"], schema.MAX_JSON_BYTES)
    page = resolve(root, site["data"]["page"], MAX_LOG)
    require(site_steps["site_binding"][0]["argv"] == download_argv(SITE_URL, binding, schema.MAX_JSON_BYTES),
            "site binding was not fetched live")
    require(site_steps["site_page"][0]["argv"] == download_argv("https://gripsack.dev/", page, MAX_LOG),
            "site page was not fetched live")
    validate_site(schema.load_json(binding), manifest, ident)
    require(version in page.read_text() and "gripsack" in page.read_text().lower(), "live site page has stale release version")
    return receipt


def public_asset_url(version, name):
    return f"https://github.com/{schema.REPOSITORY}/releases/download/core-v{version}/{name}"


def download_argv(url, path, maximum):
    return ["curl", "--disable", "--fail", "--silent", "--show-error", "--location", "--proto", "=https",
            "--proto-redir", "=https", "--max-time", "300", "--max-filesize", str(maximum), "--output", str(path), url]
