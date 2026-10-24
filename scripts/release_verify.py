#!/usr/bin/env python3
"""Observe a signed release using disposable, credential-free real consumers.

No publication, signing, global installation, caller waivers, or mock fallback.
A missing external surface produces retained pending evidence, never completion.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import subprocess
import sys
import time

import release_consumer as consumer
import release_evidence as evidence
import release_manifest as schema
import release_publish as publish
from local_artifact_consumers import EXTERNAL, snapshot
from examples_check.fixtures import install_sdk

require = schema.require


class Pending(ValueError):
    pass


def retain_source(root, directory, source, files):
    """Retain Git object inclusion proofs; never execute the nominated recipes."""
    directory.mkdir()
    env = {"PATH": os.defpath, "HOME": "/nonexistent", "LANG": "C",
           "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1",
           "GIT_NO_REPLACE_OBJECTS": "1", "GIT_NO_LAZY_FETCH": "1"}
    def git_bytes(*args):
        result = subprocess.run(["git", "-C", str(source), *args], env=env,
                                capture_output=True, timeout=60)
        require(result.returncode == 0 and len(result.stdout) <= schema.MAX_JSON_BYTES, "cannot retain bounded source object")
        return result.stdout
    def retain(kind, identifier):
        size = int(git_bytes("cat-file", "-s", identifier))
        require(0 < size <= schema.MAX_JSON_BYTES, "source object exceeds bound")
        data = git_bytes("cat-file", kind, identifier)
        path = directory / identifier
        path.write_bytes(data)
        return evidence.reference(root, path), data
    commit = schema.git(source, "rev-parse", "HEAD")
    commit_ref, raw = retain("commit", commit)
    tree = raw.split(b"\n", 1)[0].decode().removeprefix("tree ")
    result = {"commit": commit_ref, "trees": {}, "files": {}}
    if files is None:
        files = git_bytes("ls-files", "-z").decode().rstrip("\0").split("\0")
    decoded = {}
    def load_tree(identifier):
        if identifier not in decoded:
            ref, data = retain("tree", identifier)
            result["trees"][identifier] = ref
            decoded[identifier] = evidence.tree_entries(data)
        return decoded[identifier]
    for relative in files:
        parts = schema.safe_path(relative).split("/")
        current = tree
        for part in parts[:-1]:
            mode, current = load_tree(current)[part]
            require(mode == "40000", "source recipe parent is not a directory")
        mode, blob = load_tree(current)[parts[-1]]
        require(mode in {"100644", "100755"}, "source recipes must be regular files")
        ref, _ = retain("blob", blob)
        result["files"][relative] = ref
    return result


def clean_environment(root):
    home, temporary = root / "home", root / "tmp"
    home.mkdir(mode=0o700)
    temporary.mkdir(mode=0o700)
    (home / ".npmrc").write_bytes(b"")
    (home / "npm-globalrc").write_bytes(b"")
    # Deliberate allowlist, not an incomplete list of known secret names.
    environment = {"PATH": os.environ.get("PATH", os.defpath), "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8",
                   "HOME": str(home), "TMPDIR": str(temporary), "GRIPSACK_HOME": str(home / ".gs"),
                   "XDG_CONFIG_HOME": str(home / ".config"), "XDG_CACHE_HOME": str(home / ".cache"),
                   "XDG_DATA_HOME": str(home / ".local/share"), "CARGO_HOME": str(home / ".cargo"),
                   "NPM_CONFIG_USERCONFIG": str(home / ".npmrc"), "NPM_CONFIG_GLOBALCONFIG": str(home / "npm-globalrc"),
                   "NPM_CONFIG_CACHE": str(home / ".npm"), "GIT_CONFIG_GLOBAL": "/dev/null",
                   "GIT_CONFIG_NOSYSTEM": "1", "GIT_OPTIONAL_LOCKS": "0",
                   "PYTHONDONTWRITEBYTECODE": "1", "PYTEST_DISABLE_PLUGIN_AUTOLOAD": "1"}
    return environment


class Runner:
    """Wall-time and output-bounded process groups with retained actual results."""
    def __init__(self, root):
        self.root = root
        self.environment = clean_environment(root)
        (root / "logs").mkdir()
        self.number = 0

    def run(self, surface, step, argv, *, cwd=None, input=None, timeout=1800):
        argv = list(map(str, argv))
        cwd = Path(cwd or self.root)
        require(cwd.is_relative_to(self.root), "execution outside owned work directory")
        prefix = self.root / "logs" / f"{self.number:04d}"
        self.number += 1
        out_path, err_path = prefix.with_suffix(".stdout"), prefix.with_suffix(".stderr")
        stdin = None
        if input is not None:
            stdin = prefix.with_suffix(".stdin")
            stdin.write_bytes(input.encode() if isinstance(input, str) else input)
        record = {"step": step, "argv": argv, "cwd": str(cwd), "environment": dict(self.environment),
                  "returncode": None, "termination": "not_started", "duration_seconds": 0,
                  "stdout": None, "stderr": None, "stdin": evidence.reference(self.root, stdin) if stdin else None}
        surface["commands"].append(record)
        started = time.monotonic()
        process = None
        with out_path.open("xb") as out, err_path.open("xb") as err:
            try:
                with (stdin.open("rb") if stdin else open(os.devnull, "rb")) as source:
                    process = subprocess.Popen(argv, cwd=cwd, env=self.environment, stdin=source,
                                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
                    with selectors.DefaultSelector() as selector:
                        selector.register(process.stdout, selectors.EVENT_READ, out)
                        selector.register(process.stderr, selectors.EVENT_READ, err)
                        counts = {out: 0, err: 0}
                        record["termination"] = "exited"
                        while selector.get_map():
                            if time.monotonic() - started > timeout:
                                record["termination"] = "timeout"
                                raise TimeoutError(f"{step} exceeded {timeout}s")
                            for key, _ in selector.select(0.2):
                                chunk = os.read(key.fileobj.fileno(), 65536)
                                if not chunk:
                                    selector.unregister(key.fileobj)
                                    continue
                                remaining = evidence.MAX_LOG - counts[key.data]
                                key.data.write(chunk[:remaining])
                                counts[key.data] += len(chunk)
                                if counts[key.data] > evidence.MAX_LOG:
                                    record["termination"] = "output_limit"
                                    raise ValueError(f"{step} exceeded retained log bound")
                        remaining = max(0.01, timeout - (time.monotonic() - started))
                        record["returncode"] = process.wait(timeout=remaining)
            finally:
                if process is not None:
                    # Descendants retaining pipes must not outlive this owned
                    # invocation, including when their leader already exited.
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.wait()
                    record["returncode"] = process.returncode
                    process.stdout.close()
                    process.stderr.close()
                record["duration_seconds"] = round(time.monotonic() - started, 6)
                out.flush()
                err.flush()
                record["stdout"] = evidence.reference(self.root, out_path)
                record["stderr"] = evidence.reference(self.root, err_path)
        require(record["termination"] == "exited" and record["returncode"] == 0,
                f"{step} failed with rc={record['returncode']}; retained logs {prefix.name}")
        return out_path.read_text(errors="replace")

    def download(self, surface, step, url, destination, maximum):
        self.run(surface, step, evidence.download_argv(url, destination, maximum), timeout=330)
        require(destination.stat().st_size <= maximum, "download exceeds byte bound")
        return evidence.reference(self.root, destination)


class RecordingRegistry(publish.Registry):
    def __init__(self, root, data):
        super().__init__(environ={})
        self.root, self.data = root, data
        self.directory = root / "registry"
        self.directory.mkdir()

    def request(self, url, **kwargs):
        require(kwargs.get("method", "GET") == "GET" and not kwargs.get("token"), "verification is read-only")
        payload = super().request(url, **kwargs)
        if payload is None:
            raise Pending(f"registry payload not yet published: {url}")
        suffix = ".tgz" if url.endswith(".tgz") else ".crate" if url.endswith(".crate") else ".json"
        path = self.directory / f"{len(self.data['responses']):04d}{suffix}"
        path.write_bytes(payload)
        self.data["responses"].append({"url": url, "body": evidence.reference(self.root, path)})
        return payload


class Verification:
    def __init__(self, args):
        self.args = args
        self.root = args.output.absolute()
        self.root.mkdir(mode=0o700)  # Refuse reuse: fresh homes are part of the recipe.
        require(not self.root.is_symlink(), "evidence directory is an alias")
        self.runner = Runner(self.root)
        self.record = schema.load_json(args.request)
        self.receipt = {"format": evidence.FORMAT, "identity": None, "status": "pending",
                        "manifest": None, "surfaces": {}}
        self.surfaces = {}
        self.manifest = None
        self.installed = None

    def new_surface(self, name):
        result = {"format": evidence.SURFACE_FORMAT, "surface": name, "identity": self.receipt["identity"],
                  "status": "pending", "commands": [], "data": {}, "detail": None}
        self.surfaces[name] = result
        return result

    def save(self):
        for name, surface in self.surfaces.items():
            path = self.root / f"{name}.json"
            path.write_bytes(schema.canonical(surface) + b"\n")
            self.receipt["surfaces"][name] = evidence.reference(self.root, path)
        path = self.root / "completion.json"
        path.write_bytes(schema.canonical(self.receipt) + b"\n")
        return path

    def perform(self, name, operation):
        surface = self.surfaces.get(name) or self.new_surface(name)
        try:
            operation(surface)
            surface["status"] = "verified"
        except (OSError, ValueError, RuntimeError, TimeoutError, subprocess.SubprocessError, KeyError) as error:
            surface["status"] = "pending" if isinstance(error, Pending) else "unknown"
            surface["detail"] = str(error)
        self.save()

    def signed_manifest(self, surface):
        version = self.record["binding"]["tag"].removeprefix("core-v")
        from release_operator import GitHub, validate_run
        self.run = validate_run(GitHub().api(f"repos/{schema.REPOSITORY}/actions/runs/{self.args.run_id}"), self.record)
        if self.run.get("status") != "completed" or self.run.get("conclusion") != "success":
            raise Pending("signing/publishing workflow has not successfully completed")
        run_path = self.root / "workflow-run.json"
        run_path.write_bytes(schema.canonical(self.run))
        # These are trusted verifier calls, never candidate programs. Public API
        # selection and source decoding use the installer's existing implementation.
        release = consumer.select_release(version)
        self.receipt["identity"] = evidence.identity(self.record, self.args.run_id, release["id"])
        surface["identity"] = self.receipt["identity"]
        assets = consumer.release_assets(release)
        directory = self.root / "downloads"
        directory.mkdir()
        name = f"gripsack-release-{version}.json"
        require(name in assets and name + ".sigstore.json" in assets, "signed manifest not yet published")
        # Fetch setup commands are retained separately, not confused with the two
        # cryptographic verification invocations admitted by the observer.
        setup = self.new_surface("transport_setup")
        path = directory / name
        self.runner.download(setup, "manifest", evidence.public_asset_url(version, name), path, schema.MAX_JSON_BYTES)
        self.manifest = schema.validate_manifest(schema.load_json(path))
        evidence.bound_manifest(self.manifest, schema.digest(path), self.record)
        source = consumer.tag_source(release["tag_name"])
        require(source == self.manifest["source"], "release source differs")
        require(set(assets) == {Path(a["path"]).name for a in self.manifest["artifacts"]} | {name, name + ".sigstore.json"},
                "release assets inventory differs")
        self.assets = assets
        self.receipt["manifest"] = evidence.reference(self.root, path)
        signature = directory / (name + ".sigstore.json")
        self.runner.download(setup, "signature", evidence.public_asset_url(version, signature.name), signature, consumer.MAX_JSON)
        self.runner.run(setup, "verifier_version", ["gh", "--version"])
        counter = iter(("attestation_bundle", "attestation_repository"))
        consumer.verify_attestation(path, self.manifest, signature,
            command_runner=lambda args: self.runner.run(surface, next(counter), args, timeout=330))
        release_path, source_path = self.root / "release.json", self.root / "tag-source.json"
        release_path.write_bytes(schema.canonical(release))
        source_path.write_bytes(schema.canonical(source))
        require(schema.clean_source(self.args.source) == self.manifest["source"], "recipe source is not exact clean candidate")
        proof = retain_source(self.root, self.root / "source-proof", self.args.source, evidence.CORE_RECIPES)
        surface["data"] = {"bundle": evidence.reference(self.root, signature),
                           "release": evidence.reference(self.root, release_path),
                           "tag_source": evidence.reference(self.root, source_path), "source_proof": proof,
                           "workflow_run": evidence.reference(self.root, run_path)}
        # Transport setup is a retained diagnostic, not a caller-created surface.
        setup_path = self.root / "transport-setup.json"
        setup_path.write_bytes(schema.canonical(setup))
        surface["data"]["transport"] = evidence.reference(self.root, setup_path)
        del self.surfaces["transport_setup"]

    def downloaded_assets(self, surface):
        surface["data"] = {"artifacts": {}}
        for item in self.manifest["artifacts"]:
            destination = self.root / "downloads" / Path(item["path"]).name
            require(self.assets[destination.name].get("size") == item["size_bytes"], "release asset size differs")
            ref = self.runner.download(surface, item["path"], evidence.public_asset_url(self.version, destination.name),
                                       destination, item["size_bytes"])
            require(ref["sha256"] == item["sha256"] and ref["size_bytes"] == item["size_bytes"], "asset bytes differ")
            surface["data"]["artifacts"][item["path"]] = ref
        core = next(a for a in self.manifest["artifacts"] if a["role"] == "core")
        executable = self.root / "downloads/grip"
        consumer.extract_core(self.root / "downloads" / Path(core["path"]).name, self.version, executable)
        surface["data"]["core"] = evidence.reference(self.root, executable)

    @property
    def version(self):
        return self.manifest["compatible"]["core"]

    def registries(self, surface):
        surface["data"] = {"responses": [], "packages": {}}
        registry = RecordingRegistry(self.root, surface["data"])
        for package in self.manifest["packages"]:
            require(registry.remote_digest(package) == package["sha256"], "registry payload not yet visible")
            surface["data"]["packages"][package["path"]] = surface["data"]["responses"][-1]["body"]

    def installer(self, surface):
        source = self.args.source.resolve()
        require(schema.clean_source(source) == self.manifest["source"], "consumer recipe source is not exact clean candidate")
        installer = self.root / "install.sh"
        shutil.copyfile(schema.regular(source / "install.sh"), installer)
        self.runner.run(surface, "install", ["sh", installer, "--version", self.version], timeout=3600)
        installed = Path(self.runner.environment["HOME"]) / ".local/bin/grip"
        expected = self.surfaces["downloaded_assets"]["data"]["core"]["sha256"]
        require(schema.digest(installed) == expected, "installer wrote wrong executable")
        require(self.runner.run(surface, "installed_version", [installed, "--version"]).strip() == f"grip {self.version}",
                "installed executable version differs")
        self.installed = installed
        surface["data"] = {"installer": evidence.reference(self.root, installer),
                           "installed": evidence.reference(self.root, self.installed)}
        baseline = self.root / "update-baseline"
        baseline.mkdir()
        if self.args.previous_version:
            previous = consumer.stable_version("core-v" + self.args.previous_version)
            require(previous is not None and previous < consumer.stable_version("core-v" + self.version),
                    "updater baseline must precede candidate")
            # Never execute an unsigned historical binary as a release baseline.
            consumer.admit(baseline, self.args.previous_version)
            original = baseline / "grip"
            mode, previous_version = "upgrade", self.args.previous_version
        else:
            original = self.installed
            mode, previous_version = "reinstall", self.version
        updater = self.root / "updater/grip"
        updater.parent.mkdir()
        shutil.copyfile(original, updater)
        updater.chmod(0o755)
        before = self.root / "before-update-grip"
        os.link(updater, before)  # Keep the old inode alive; reuse cannot mimic replacement.
        surface["data"].update(before_update=evidence.reference(self.root, before),
                               previous_version=previous_version, mode=mode)
        args = [updater, "self-update"]
        if mode == "reinstall":
            args += ["--reinstall", "--version", self.version]
        self.runner.run(surface, "update", args, timeout=3600)
        require(schema.digest(updater) == expected, "embedded updater did not install nominated candidate bytes")
        require((before.stat().st_dev, before.stat().st_ino) != (updater.stat().st_dev, updater.stat().st_ino),
                "embedded updater was a no-op; executable was not replaced")
        self.runner.run(surface, "updated_version", [updater, "--version"])
        surface["data"]["after_update"] = evidence.reference(self.root, updater)

    def approve(self, surface, prefix, repo):
        inspected = json.loads(self.runner.run(surface, prefix + "_inspect",
                                              [self.installed, "trust", "inspect", repo, "--json"], cwd=repo))
        require(Path(inspected["repository"]) == repo, "approval repository differs")
        self.runner.run(surface, prefix + "_approve", [self.installed, "trust", "add", repo,
                        "--bundle", inspected["bundle_digest"], "--policy", inspected["policy_digest"]], cwd=repo)

    def example(self):
        source = self.args.example.resolve()
        require(schema.git(source, "rev-parse", "HEAD") == EXTERNAL["example"][1], "external recipe is not frozen qualified revision")
        require(not schema.git(source, "status", "--porcelain", "--untracked-files=all", "--ignored"), "external recipe is dirty")
        return source

    def sdk(self, surface):
        require(self.installed is not None, "installer has not produced a verified executable")
        package = next(p for p in self.manifest["packages"] if p["registry"] == "npm")
        packed = evidence.resolve(self.root, self.surfaces["registries"]["data"]["packages"][package["path"]])
        metadata = publish.npm_metadata(packed)
        require(metadata.get("name") == package["name"] and metadata.get("version") == package["version"],
                "packed SDK metadata differs from compatible release")
        project = self.root / "sdk-consumer"
        shutil.copytree(self.example(), project, ignore=shutil.ignore_patterns(".git"))
        self.runner.run(surface, "npm_install", ["npm", "install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund",
                                                "--package-lock=false", "--save-exact", packed], cwd=project)
        installed = project / "node_modules/@gripsack/core"
        extracted = self.root / "packed-sdk"
        schema.extract_archive(packed, extracted, maximum=publish.MAX_EXPANDED)
        require({p: value["sha256"] for p, value in snapshot(installed).items()}
                == {p: value["sha256"] for p, value in snapshot(extracted / "package").items()},
                "installed npm SDK differs from packed bytes")
        self.sdk_root = installed
        self.approve(surface, "sdk", project)
        output = self.runner.run(surface, "sdk_check", [self.installed, "check", "--host", "laptop", "--json"], cwd=project)
        evidence.check_output(output, legacy=True)
        surface["data"] = {"tarball": evidence.reference(self.root, packed),
                           "package_root": str(installed.relative_to(self.root)),
                           "installed_files": {str(p.relative_to(installed)): evidence.reference(self.root, p)
                                               for p in installed.rglob("*") if p.is_file()},
                           "recipe": evidence.reference(self.root, project / "hosts/laptop.ts"),
                           "source_proof": retain_source(self.root, self.root / "example-proof", self.example(), None)}

    def default_helpers(self, surface):
        require(self.installed is not None and hasattr(self, "sdk_root"), "installed core/packed SDK not available")
        conda = self.root / "conda-consumer"
        shutil.copytree(self.example() / "workspaces/conda", conda)
        lock = conda / "gripsack.lock"
        original_lock = schema.digest(lock)
        home = Path(self.runner.environment["GRIPSACK_HOME"])
        require(not any((home / "tools" / f"{stem}-{self.version}").exists()
                        for stem in ("conda-helper", "buildkit-bridge")), "helper cache must be cold before default provisioning")
        self.approve(surface, "conda", conda)
        output = self.runner.run(surface, "conda_check", [self.installed, "check", "--json"], cwd=conda)
        evidence.check_output(output)
        built = json.loads(self.runner.run(surface, "conda_build", [self.installed, "build", "ripgrep", "--json"], cwd=conda))
        result = self.runner.run(surface, "conda_run", [self.installed, "run", "--env", "search", "--", "rg", "^needle$"],
                                 cwd=conda, input="other\nneedle\n")
        require(result.strip() == "needle" and schema.digest(lock) == original_lock, "frozen Conda consumer changed or did not execute")
        surface["data"] = {"installed": {}, "conda_lock": evidence.reference(self.root, lock),
                           "conda_output": evidence.reference(self.root, Path(built["outputs"][0]["path"]) / "bin/rg")}
        bridge = self.root / "bridge-consumer"
        shutil.copytree(self.args.source / "examples/workspaces/03-source-built", bridge)
        install_sdk(bridge, self.sdk_root)
        self.approve(surface, "bridge", bridge)
        # This is the checked-in real static C build, not an echo fixture. The
        # core owns its BuildKit worker under this fresh private GRIPSACK_HOME.
        try:
            self.runner.run(surface, "bridge_lock", [self.installed, "update"], cwd=bridge)
            built = json.loads(self.runner.run(surface, "bridge_build", [self.installed, "build", "greeter", "--json"], cwd=bridge))
            self.runner.run(surface, "bridge_run", [self.installed, "task", "run", "smoke"], cwd=bridge)
            surface["data"]["bridge_output"] = evidence.reference(self.root, Path(built["outputs"][0]["path"]) / "bin/greeter")
        finally:
            self.runner.run(surface, "bridge_stop", [self.installed, "builder", "stop"], cwd=bridge)
        for role, filename in (("conda", "gripsack-conda"), ("bridge", "bridge")):
            matches = [p for p in (home / "tools").rglob(filename) if p.is_file()]
            require(len(matches) == 1, f"{role} default helper was not provisioned")
            require(schema.digest(matches[0]) == self.manifest["compatible"]["helper_pins"][role], "default helper bytes differ")
            surface["data"]["installed"][role] = evidence.reference(self.root, matches[0])

    def site(self, surface):
        binding, page = self.root / "site-release.json", self.root / "site.html"
        surface["data"] = {"url": evidence.SITE_URL,
                           "binding": self.runner.download(surface, "site_binding", evidence.SITE_URL, binding, schema.MAX_JSON_BYTES),
                           "page": self.runner.download(surface, "site_page", "https://gripsack.dev/", page, evidence.MAX_LOG)}
        try:
            evidence.validate_site(schema.load_json(binding), self.manifest, self.receipt["identity"])
            require(self.version in page.read_text() and "gripsack" in page.read_text().lower(), "site page release is stale")
        except ValueError as error:
            raise Pending(str(error)) from error

    def execute(self):
        self.perform("signed_manifest", self.signed_manifest)
        if self.surfaces["signed_manifest"]["status"] != "verified":
            return self.save()
        for name in evidence.required_surfaces(self.manifest) - {"signed_manifest"}:
            self.new_surface(name)
        for name in ("downloaded_assets", "registries", "installer", "sdk", "default_helpers", "site"):
            self.perform(name, getattr(self, name))
        if all(surface["status"] == "verified" for surface in self.surfaces.values()):
            self.receipt["status"] = "complete"
            path = self.save()
            try:
                evidence.verify_completion(path, schema.digest(path), self.record, self.run)
            except (OSError, ValueError, KeyError, TypeError) as error:
                self.receipt["status"] = "unknown"
                self.surfaces["installer"]["status"] = "unknown"
                self.surfaces["installer"]["detail"] = f"retained evidence admission: {error}"
        return self.save()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--request", type=Path, required=True, help="durable release_operator request JSON")
    parser.add_argument("--run-id", type=int, required=True)
    parser.add_argument("--source", type=Path, required=True, help="exact clean frozen source checkout")
    parser.add_argument("--example", type=Path, required=True, help="qualified pinned example-env-typescript checkout")
    parser.add_argument("--output", type=Path, required=True, help="new retained private evidence directory")
    parser.add_argument("--previous-version", help="prior signed core baseline for actual embedded updater replacement")
    args = parser.parse_args(argv)
    try:
        from release_operator import Requests, host_only
        host_only()
        store = Requests(args.request.parent)
        with store.locked(args.request.stem):
            require(store.load(args.request.stem) is not None, "unknown durable release request")
        verifier = Verification(args)
        path = verifier.execute()
        print(json.dumps({"status": verifier.receipt["status"], "receipt": str(path), "sha256": schema.digest(path)}, sort_keys=True))
        return 0 if verifier.receipt["status"] == "complete" else 3
    except (OSError, ValueError, RuntimeError, KeyError, subprocess.SubprocessError) as error:
        parser.exit(2, f"release verification: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
