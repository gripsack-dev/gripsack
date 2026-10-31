"""Required offline native journeys for shared DAGs, contextual identity and SCCs.

These use real signed bundles, source.package acquisition, retained launchers and
native execution. Repeated children must remain bounded through actual projection,
not merely through catalog inspection.
"""
from __future__ import annotations

import itertools
import json
from pathlib import Path
import platform
import shutil
import subprocess

import pytest

from test_package_environment import (
    LITERAL, authority, consumer_workspace, native_source, offline_environment,
    pack_environment, project, specification, trust_environment,
)
from test_package_sharing import canonical, checked, package, recipient

pytestmark = pytest.mark.skipif(
    platform.system() != "Linux" or platform.machine() != "x86_64",
    reason="native Linux x86_64 portable projection contract",
)


def external(name, identity, runtime=()):
    return {"id": name, "payload": None, "tree": None, "runtime": list(runtime),
            "conda": None, "inventory": [], "external": {
                "manifest": identity, "namespace": "fixture/environment",
                "publisher": "fixture-publisher",
            }}


def local(name, runtime=()):
    return {"id": name, "payload": None, "tree": None, "runtime": list(runtime),
            "conda": None, "external": None, "inventory": []}


def receipts(state):
    return {path: path.read_bytes() for path in (state / "store").rglob("package.json")}


def profile_shell(state, command):
    return ["/bin/sh", "-c", '. "$1"; exec "$2" profile-argument',
            "projection-fixture", str(state / "current/env/profile.sh"), command]


@pytest.mark.parametrize("nodes,export_command", [(26, False), (4, True)])
def test_repeated_child_dag_projects_once_per_manifest(sandbox, nodes, export_command):
    producer = sandbox / "publisher"
    producer_env = recipient(producer)
    _, seed, public = authority(producer / "authority")
    trust_environment(producer_env, public)
    source, original = native_source(producer)
    identities = []
    for depth in range(nodes):
        spec = specification()
        if depth:
            spec["components"] = [external(name, identities[-1]) for name in ("left", "right")]
            spec["roots"] = ["left", "right"]
            spec["environment"] = {}
            spec["commands"] = {"native-child": {
                "component": "left", "executable": "bin/native-child", "args": [],
            }}
        if not export_command:
            spec["commands"] = {}
            spec["environment"] = {}
        bundle = sandbox / ("shared-dag-" + str(depth))
        identities.append(pack_environment(producer_env, spec, source, bundle, seed))
        package(producer_env, "import", "--bundle", bundle)
        if depth:
            shutil.rmtree(sandbox / ("shared-dag-" + str(depth - 1)))
    shutil.rmtree(producer)

    env = offline_environment(sandbox / "recipient", public)
    state = Path(env["GRIPSACK_HOME"])
    package(env, "import", "--bundle", bundle)
    package(env, "inspect", "--manifest", identities[-1])
    shutil.rmtree(bundle)
    commands = ["native-child"] if export_command else []
    repo = consumer_workspace(Path(env["HOME"]), identities[-1], commands)
    project(env, repo, "update")
    # Empty commands are a real acquisition/profile activation, not an excuse
    # to reject or skip the 26-node graph (which has 2**25 expanded leaf paths).
    project(env, repo, "apply", "selected")

    def projections():
        return {path: (path.read_bytes(), path.stat().st_ino, path.stat().st_mtime_ns)
                for path in (state / "store").rglob("projection.json")
                if path.parent.name.endswith("-package-environment-projection")}

    projected = projections()
    assert len(projected) == nodes
    records = [json.loads(data[0]) for data in projected.values()]
    assert {record["manifest"] for record in records} == set(identities)
    assert {record["authority"] for record in records} == {identities[-1]}
    # One native leaf, two views per nonleaf, and the consumer's root package.
    assert len(receipts(state)) == 2 * nodes
    immutable = receipts(state)
    if export_command:
        result = project(env, repo, "run", "--env", "dev", "--", "native-child", "shared")
        assert result.stdout == f"{LITERAL}\nrecipient-local\nfrom-manifest\nshared\n"
    project(env, repo, "apply", "selected")
    project(env, repo, "gc")
    package(env, "inspect", "--manifest", identities[-1])
    assert projections() == projected
    assert receipts(state) == immutable
    assert any(path.read_bytes() == original for path in (state / "store").rglob("native-probe"))


@pytest.mark.parametrize("order", list(itertools.permutations("ABC")))
def test_nested_child_shared_by_two_parents_and_standalone(sandbox, order):
    producer = sandbox / "publisher"
    producer_env = recipient(producer)
    _, seed, public = authority(producer / "authority")
    trust_environment(producer_env, public)
    source, original = native_source(producer)
    child = specification()
    # This admitted local self-cycle must work inside parents and standalone.
    child["components"][0]["runtime"] = ["native"]
    bundles = {name: sandbox / ("bundle-" + name) for name in "ABC"}
    identities = {"C": pack_environment(producer_env, child, source, bundles["C"], seed)}
    package(producer_env, "import", "--bundle", bundles["C"])
    for name in "AB":
        parent = specification()
        # The parent SCC itself contains an independently signed Environment;
        # interception must retain this scope while resolving the child's leaf.
        parent["components"] = [external("child", identities["C"], ["native"]),
                                local("native", ["child"])]
        parent["roots"] = ["child"]
        parent["commands"] = {"native-child": {
            "component": "child", "executable": "bin/native-child", "args": [name],
        }}
        parent["environment"] = {"PORTABLE_LITERAL": {"kind": "literal", "value": name}}
        identities[name] = pack_environment(producer_env, parent, source, bundles[name], seed)
    shutil.rmtree(producer)

    env = offline_environment(sandbox / "recipient", public)
    home, state = Path(env["HOME"]), Path(env["GRIPSACK_HOME"])
    repos = {}
    immutable = {}
    for name in order:
        package(env, "import", "--bundle", bundles[name])
        shutil.rmtree(bundles[name])
        repo = consumer_workspace(home / name, identities[name], ["native-child"])
        repos[name] = repo
        project(env, repo, "update")
        result = project(env, repo, "run", "--env", "dev", "--", "native-child", "recipient")
        expected = f"{LITERAL}\nrecipient-local\nfrom-manifest\nrecipient\n" if name == "C" else (
            f"{name}\nrecipient-local\nfrom-manifest\n{name}\nrecipient\n"
        )
        assert result.stdout == expected
        current = receipts(state)
        assert immutable.items() <= current.items(), "acquisition rewrote an existing immutable receipt"
        immutable = current

    project(env, repos["A"], "apply", "selected")
    shell = profile_shell(state, "native-child")
    assert checked(shell, env=env, text=True).stdout == "A\nrecipient-local\nfrom-manifest\nA\nprofile-argument\n"
    project(env, repos["A"], "gc")
    assert any(path.read_bytes() == original for path in (state / "store").rglob("native-probe"))
    assert immutable.items() <= receipts(state).items()
    for name in "ABC":
        project(env, repos[name], "run", "--env", "dev", "--", "native-child", "after-gc")
        package(env, "inspect", "--manifest", identities[name])

    policy_path = state / "sharing/trust.json"
    policy = json.loads(policy_path.read_bytes())
    # Parent authority cannot disappear merely because the same child was also
    # acquired in a separately authorized context. Unrelated contexts still work.
    for revoked, blocked in (("A", {"A"}), ("C", set("ABC"))):
        policy["revoked_manifests"] = [identities[revoked]]
        policy_path.write_bytes(canonical(policy))
        for name in "ABC":
            result = project(env, repos[name], "run", "--env", "dev", "--", "native-child",
                             "after-revocation", ok=name not in blocked)
            if name in blocked:
                assert "after-revocation" not in result.stdout
        result = subprocess.run(shell, env=env, capture_output=True, text=True, timeout=120)
        assert result.returncode != 0 and "profile-argument" not in result.stdout
        assert immutable.items() <= receipts(state).items()
    policy["revoked_manifests"] = []
    policy_path.write_bytes(canonical(policy))
    checked(shell, env=env, text=True)


@pytest.mark.parametrize("roots", [("zroot", "aroot"), ("aroot", "zroot")])
def test_nonroot_nested_export_keeps_signed_root_contribution_order(sandbox, roots):
    producer = sandbox / "publisher"
    producer_env = recipient(producer)
    _, seed, public = authority(producer / "authority")
    trust_environment(producer_env, public)
    source, _ = native_source(producer)
    identities = {}
    for name in ("zroot", "aroot", "tool"):
        child = specification()
        child["environment"]["PORTABLE_LITERAL"]["value"] = name
        if name == "tool":
            child["environment"]["TOOL_ONLY"] = {"kind": "literal", "value": "child"}
        bundle = producer / ("bundle-" + name)
        identities[name] = pack_environment(producer_env, child, source, bundle, seed)
        package(producer_env, "import", "--bundle", bundle)
    parent = specification()
    parent["components"] = [external(name, identities[name], ["tool"] if name == roots[0] else [])
                            for name in ("aroot", "tool", "zroot")]
    parent["roots"] = list(roots)
    parent["environment"] = {}
    parent["commands"] = {"native-child": {
        "component": "tool", "executable": "bin/native-child", "args": ["parent-fixed", ""],
    }}
    bundle = sandbox / "nonroot-export"
    identity = pack_environment(producer_env, parent, source, bundle, seed)
    selected_bundle = sandbox / "explicitly-selected-tool"
    selected = dict(parent, roots=[*roots, "tool"])
    selected_identity = pack_environment(producer_env, selected, source, selected_bundle, seed)
    shutil.rmtree(producer)
    env = offline_environment(sandbox / "recipient", public)
    repo = consumer_workspace(Path(env["HOME"]), identity, ["native-child"])
    package(env, "import", "--bundle", bundle)
    shutil.rmtree(bundle)
    project(env, repo, "update")
    expected = f"{roots[-1]}\nrecipient-local\nfrom-manifest\nparent-fixed\n\n"
    result = project(env, repo, "run", "--env", "dev", "--", "native-child", "recipient", "")
    assert result.stdout == expected + "recipient\n\n"
    assert "tool-only=" not in result.stdout
    project(env, repo, "apply", "selected")
    state = Path(env["GRIPSACK_HOME"])
    shell = profile_shell(state, "native-child")
    assert checked(shell, env=env, text=True).stdout == expected + "profile-argument\n"
    project(env, repo, "gc")
    assert checked(shell, env=env, text=True).stdout == expected + "profile-argument\n"
    # The variable is not globally filtered: selecting its owner deliberately
    # grants that contribution, after the previously selected root values.
    package(env, "import", "--bundle", selected_bundle)
    shutil.rmtree(selected_bundle)
    selected_repo = consumer_workspace(Path(env["HOME"]) / "selected-contribution",
                                       selected_identity, ["native-child"])
    project(env, selected_repo, "update")
    selected_result = project(env, selected_repo, "run", "--env", "dev", "--",
                              "native-child", "selected", "")
    assert selected_result.stdout == (
        "tool\nrecipient-local\nfrom-manifest\nparent-fixed\n\n"
        "selected\n\ntool-only=child\n"
    )


def entry_assembly(symbol, message):
    # No libc or host runtime search grants: only the selected GNU interpreter
    # and the explicitly packaged libraries satisfy this dynamic executable.
    escaped_message = message.replace("\n", "\\n")
    return f'''
.global _start
.text
_start:
    call {symbol}@PLT
    cmp $42, %eax
    jne failed
    mov $1, %eax
    mov $1, %edi
    lea message(%rip), %rsi
    mov ${len(message)}, %edx
    syscall
    xor %edi, %edi
    jmp done
failed:
    mov $90, %edi
done:
    mov $60, %eax
    syscall
.section .rodata
message: .ascii "{escaped_message}"
.section .note.GNU-stack,"",@progbits
'''


@pytest.mark.parametrize("cycle", ["self", "pair", "nested"])
def test_runtime_scc_runs_with_member_and_outgoing_libraries_after_retention(sandbox, cycle):
    producer = sandbox / "publisher"
    producer_env = recipient(producer)
    _, seed, public = authority(producer / "authority")
    source = producer / "source"
    compiler = shutil.which("cc")
    assert compiler, "native SCC qualification requires the C compiler and GNU linker"
    members = ("app", "leaf") if cycle == "self" else ("app", "peer", "leaf")
    for name in members:
        for directory in ("bin", "lib"):
            (source / "components" / name / directory).mkdir(parents=True)

    def library(member, text, dependencies=()):
        program = producer / (member + ".c")
        program.write_text(text)
        output = source / "components" / member / "lib" / ("lib" + member + ".so")
        flags = []
        for dependency in dependencies:
            flags.extend(["-L" + str(source / "components" / dependency / "lib"), "-l" + dependency])
        checked([compiler, "-nostdlib", "-shared", "-fPIC", program, *flags,
                 "-Wl,-soname,lib" + member + ".so", "-o", output])

    library("leaf", "int leaf_value(void) { return 42; }\n")
    library("app", "extern int leaf_value(void); int app_value(void) { return leaf_value(); }\n", ["leaf"])
    if cycle != "self":
        library("peer", "extern int leaf_value(void); int peer_value(void) { return leaf_value(); }\n", ["leaf"])
    commands = {}
    for member in members:
        if member == "leaf":
            continue
        dependency = "peer" if cycle != "self" and member == "app" else "app"
        program = producer / (member + ".S")
        program.write_text(entry_assembly(dependency + "_value", member + "=42\n"))
        binary = source / "components" / member / "bin/probe"
        checked([compiler, "-nostdlib", "-no-pie", program,
                 "-L" + str(source / "components" / dependency / "lib"), "-l" + dependency,
                 "-Wl,-rpath-link," + str(source / "components/leaf/lib"), "-o", binary])
        commands["probe-" + member] = {"component": member, "executable": "bin/probe", "args": []}
    spec = specification()
    spec["target"]["abi"] = {"kind": "glibc", "minimum": "2.17"}
    spec["components"] = ([local("app", ["app", "leaf"]), local("leaf")] if cycle == "self" else
                          [local("app", ["peer"]), local("leaf"), local("peer", ["app", "leaf"])])
    spec["roots"] = ["app"]
    spec["commands"] = commands
    spec["environment"] = {}
    children = {}
    if cycle == "nested":
        trust_environment(producer_env, public)
        for member in ("app", "peer"):
            child = specification()
            child["target"] = spec["target"]
            child["components"] = ([local("app", ["app"])] if member == "app" else
                                   [local("leaf"), local("peer", ["leaf"])])
            child["roots"] = [member]
            child["commands"] = {"probe-" + member: commands["probe-" + member]}
            child["environment"] = {}
            transfer = sandbox / ("nested-" + member)
            children[member] = (pack_environment(producer_env, child, source, transfer, seed), transfer)
            # Passive import must not execute or demand standalone ELF closure.
            package(producer_env, "import", "--bundle", transfer)
        spec["components"] = [external("app", children["app"][0], ["peer"]),
                              external("peer", children["peer"][0], ["app"])]
        spec["commands"] = {
            name: {"component": command["component"], "executable": "bin/" + name, "args": []}
            for name, command in commands.items()
        }
    bundle = sandbox / "cycle-bundle"
    identity = pack_environment(producer_env, spec, source, bundle, seed)
    shutil.rmtree(producer)
    env = offline_environment(sandbox / "recipient", public)
    repo = consumer_workspace(Path(env["HOME"]), identity, commands)
    package(env, "import", "--bundle", bundle)
    if children:
        # Independently incomplete children must not gain a stale admitted bit
        # from their successful enclosing SCC. Both acquisition orders matter.
        for member, (child_identity, transfer) in children.items():
            package(env, "import", "--bundle", transfer)
            shutil.rmtree(transfer)
            child_repo = consumer_workspace(Path(env["HOME"]) / member, child_identity, ["probe-" + member])
            project(env, child_repo, "update", ok=False)
    shutil.rmtree(bundle)
    project(env, repo, "update")
    for command in commands:
        result = project(env, repo, "run", "--env", "dev", "--", command)
        assert result.stdout == command.removeprefix("probe-") + "=42\n"
    project(env, repo, "apply", "selected")
    project(env, repo, "gc")
    state = Path(env["GRIPSACK_HOME"])
    saved = receipts(state)
    for command in commands:
        assert checked(profile_shell(state, command), env=env, text=True).stdout == command.removeprefix("probe-") + "=42\n"
    assert saved.items() <= receipts(state).items()
    if children:
        for member, (child_identity, _) in children.items():
            child_repo = Path(env["HOME"]) / member / "local project"
            project(env, child_repo, "update", ok=False)
    policy_path = state / "sharing/trust.json"
    policy = json.loads(policy_path.read_bytes())
    for revoked in [identity, *(child_identity for child_identity, _ in children.values())]:
        policy["revoked_manifests"] = [revoked]
        policy_path.write_bytes(canonical(policy))
        for command in commands:
            project(env, repo, "run", "--env", "dev", "--", command, ok=False)
            result = subprocess.run(profile_shell(state, command), env=env,
                                    capture_output=True, text=True, timeout=120)
            assert result.returncode != 0 and "=42" not in result.stdout
        assert saved.items() <= receipts(state).items()
    policy["revoked_manifests"] = []
    policy_path.write_bytes(canonical(policy))
    for command in commands:
        assert checked(profile_shell(state, command), env=env, text=True).stdout == command.removeprefix("probe-") + "=42\n"
