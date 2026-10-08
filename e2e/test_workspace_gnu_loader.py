"""Actual sealed GNU launches: run on both UBI8.10 and modern GNU userspace.

The same tests use each host's real loader/libraries, not a forged libc fact.
A UBI8 container run does not qualify the reviewer's external el9 kernel.
"""
import json
import platform
import re
import struct
import subprocess
from pathlib import Path

import pytest

from conftest import GRIP, grip, make_toolchain_tarball, run_grip

pytestmark = pytest.mark.skipif(
    platform.system() != "Linux" or platform.machine() != "x86_64",
    reason="native Linux/x86_64 ELF fixture",
)


def shell_payload():
    shell = Path("/bin/sh").resolve()
    inventory = subprocess.run(
        ["ldd", str(shell)], capture_output=True, text=True, check=True,
    )
    files = {"bin/probe": shell.read_bytes()}
    # Trusted host shell only: ldd is never run on repository-selected input.
    libraries = re.findall(r"(?:=>\s+)?(/[^\s(]+)", inventory.stdout)
    assert libraries, inventory.stdout
    for library in libraries:
        path = Path(library)
        files["lib/" + path.name] = path.read_bytes()
    return files


def declare(sandbox, files, *, abi="gnu", environment_entries=None):
    archive = make_toolchain_tarball(sandbox / "native.tar.gz", files)
    repo = sandbox / "workspace"
    repo.mkdir()
    target = '{os:"linux",arch:"x86_64"' + (',abi:"gnu"}' if abi else '}')
    environment = ",env:{" + ",".join(
        f"{json.dumps(key)}:lit({json.dumps(value)})"
        for key, value in (environment_entries or {}).items()
    ) + "}"
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, pkg, provider, tarball, environment, lit,
    } from "@gripsack/core";
    export default defineWorkspace(() => {
      const target = ''' + target + ''';
      return workspace({outputs:[
        pkg("tools",{producer:provider(tarball(''' + json.dumps(archive.as_uri()) + ''')),
          commands:{probe:"bin/probe"},target,layout:{kind:"relocatable"}}),
        environment("dev",{packages:["tools"],target''' + environment + '''}),
      ]});
    });
    ''')
    updated = grip("update", "tools", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    return repo


def test_gnu_sealed_loader_preserves_argv_and_ignores_ambient_loader_inputs(sandbox, monkeypatch):
    repo = declare(sandbox, shell_payload())
    built = grip("build", "tools", "--json", cwd=repo)
    assert built.returncode == 0, built.stdout + built.stderr
    retained = Path(json.loads(built.stdout)["outputs"][0]["path"])
    for key, value in {
        "LD_PRELOAD": "/nonexistent/unadmitted-preload.so",
        "LD_AUDIT": "/nonexistent/unadmitted-audit.so",
        "LD_LIBRARY_PATH": str(sandbox / "hostile"),
        "LD_HWCAP_MASK": "0xffffffffffffffff",
        "GLIBC_TUNABLES": "glibc.cpu.hwcaps=hostile",
    }.items():
        monkeypatch.setenv(key, value)
    result = grip("run", "--env", "dev", "--", "probe", "-c",
                  'printf "%s\\n" "$0"; '
                  'test -z "${LD_PRELOAD+x}${LD_AUDIT+x}${LD_HWCAP_MASK+x}${GLIBC_TUNABLES+x}"', cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    assert result.stdout.strip() == str(retained / "bin/probe")
    arguments = grip("run", "--env", "dev", "--", "probe", "-c",
                     'printf "%s|%s|%s" "$0" "$1" "$2"',
                     "chosen shell name", "argument with spaces", "", cwd=repo)
    assert arguments.returncode == 0, arguments.stdout + arguments.stderr
    assert arguments.stdout == "chosen shell name|argument with spaces|"


@pytest.mark.parametrize("capability", ["tls", "x86_64", "haswell", "avx512_1"])
def test_gnu_legacy_hwcaps_cannot_shadow_flat_dependency_admission(sandbox, capability):
    files = shell_payload()
    library = next(name for name in files if name.startswith("lib/libc.so"))
    files[f"lib/{capability}/libc.so.6"] = files[library]
    repo = declare(sandbox, files)
    result = grip("run", "--env", "dev", "--", "probe", "-c", "printf unadmitted", cwd=repo)
    assert result.returncode != 0
    assert "E128" in result.stderr and "capability" in result.stderr
    assert "unadmitted" not in result.stdout


@pytest.mark.parametrize("key", ["LD_PRELOAD", "LD_AUDIT", "LD_HWCAP_MASK", "GLIBC_TUNABLES"])
def test_declared_loader_inputs_cannot_bypass_gnu_closure(sandbox, key):
    repo = declare(sandbox, shell_payload(), environment_entries={key: "/nonexistent/unadmitted.so"})
    result = grip("run", "--env", "dev", "--", "probe", "-c", "printf escaped", cwd=repo)
    assert result.returncode != 0
    assert "E128" in result.stderr and key in result.stderr
    assert "escaped" not in result.stdout


def test_gnu_hwcaps_mask_controls_actual_library_lookup(sandbox):
    files = shell_payload()
    for level in ("x86-64-v2", "x86-64-v3", "x86-64-v4"):
        files[f"lib/glibc-hwcaps/{level}/libc.so.6"] = b"not the admitted libc image"
    repo = declare(sandbox, files)
    result = grip("run", "--env", "dev", "--", "probe", "-c", "printf admitted", cwd=repo)
    assert result.returncode == 0, result.stdout + result.stderr
    assert result.stdout == "admitted"


def test_dynamic_native_execution_still_requires_explicit_abi(sandbox):
    repo = declare(sandbox, shell_payload(), abi=None)
    result = grip("run", "--env", "dev", "--", "probe", "-c", "printf escaped", cwd=repo)
    assert result.returncode != 0
    assert "E128" in result.stderr and "ABI" in result.stderr
    assert "escaped" not in result.stdout


def test_static_native_execution_does_not_require_gnu_loader_controls(sandbox):
    # A complete freestanding x86_64 ELF: exit(37), no PT_INTERP or libc.
    instructions = b"\xb8\x3c\x00\x00\x00\xbf\x25\x00\x00\x00\x0f\x05"
    size = 64 + 56 + len(instructions)
    header = struct.pack("<16sHHIQQQIHHHHHH", b"\x7fELF\x02\x01\x01" + b"\0" * 9,
                         2, 62, 1, 0x400000 + 120, 64, 0, 0, 64, 56, 1, 0, 0, 0)
    segment = struct.pack("<IIQQQQQQ", 1, 5, 0, 0x400000, 0x400000, size, size, 4096)
    repo = declare(sandbox, {"bin/probe": header + segment + instructions}, abi=None)
    result = grip("run", "--env", "dev", "--", "probe", cwd=repo)
    assert result.returncode == 37, result.stdout + result.stderr


def test_host_runtime_projected_environment_survives_nested_and_profile_launch(sandbox, monkeypatch):
    libraries = sandbox / "reviewed-host-libraries"
    libraries.mkdir()
    library = libraries / "libreview.so"
    source = sandbox / "library.c"
    source.write_text("int reviewed(void) { return 42; }\n")
    subprocess.run(["/usr/bin/cc", "-shared", "-fPIC", "-Wl,-soname,libreview.so",
                    str(source), "-o", str(library)], check=True, capture_output=True)
    program = sandbox / "probe"
    source.write_text(r'''
        #include <stdio.h>
        #include <stdlib.h>
        #include <string.h>
        extern int reviewed(void);
        static const char *value(const char *name) {
            const char *v = getenv(name); return v ? v : "<unset>";
        }
        int main(int argc, char **argv) {
            size_t bulk = 0;
            for (int i = 0; i < 8; ++i) {
                char name[16]; snprintf(name, sizeof(name), "BULK%d", i);
                const char *v = getenv(name); if (v) bulk += strlen(v);
            }
            printf("%s|%s|%s|%s|%s|%d|%zu", value("APP_CONFIG"), value("EMPTY"),
                   value("TASK_ONLY"), value("UNDECLARED"), value("CONDA_PREFIX"),
                   reviewed(), bulk);
            for (int i = 1; i < argc; ++i) printf("<%s>", argv[i]);
            if (argc > 1 && !strcmp(argv[1], "stdin")) {
                char data[64];
                if (!fgets(data, sizeof(data), stdin)) return 9;
                printf("|input=%s", data);
            } else putchar('\n');
            return argc > 1 && !strcmp(argv[1], "fail") ? 7 : 0;
        }
    ''')
    subprocess.run(["/usr/bin/cc", str(source), str(library),
                    f"-Wl,-rpath,{libraries}", "-o", str(program)],
                   check=True, capture_output=True)
    archive = make_toolchain_tarball(sandbox / "host-runtime.tar.gz",
                                     {"bin/probe": program.read_bytes()})
    repo = sandbox / "host-runtime-env"
    repo.mkdir()
    declared = {"APP_CONFIG": "spaces '$HOME'", "EMPTY": "", "CONDA_PREFIX": "/reviewed prefix"}
    declared.update({f"BULK{i}": "x" * 24000 for i in range(8)})
    environment = ",".join(f"{json.dumps(key)}:lit({json.dumps(value)})"
                           for key, value in declared.items())
    (repo / "gripsack.ts").write_text('''import {
      defineWorkspace, workspace, pkg, provider, tarball, environment, profile,
      task, check, exec, lit,
    } from "@gripsack/core";
    export default defineWorkspace(() => {
      const target = {os:"linux",arch:"x86_64",abi:"gnu"} as const;
      return workspace({outputs:[
        pkg("tools",{producer:provider(tarball(''' + json.dumps(archive.as_uri()) + ''')),
          commands:{probe:"bin/probe"},target,layout:{kind:"relocatable"},
          hostRuntime:{libraryDirectories:[''' + json.dumps(str(libraries)) + ''']}}),
        environment("dev",{packages:["tools"],target,env:{''' + environment + '''}}),
        profile("personal",{environment:"dev"}),
        check("after",{subject:"tools",run:exec({
          argv:[lit("/bin/sh"),lit("-c"),lit("exec probe")],
          env:{TASK_ONLY:lit("check overlay")}
        })}),
        task("nested",{environment:"dev",steps:[exec({
          argv:[lit("/bin/sh"),lit("-c"),lit("exec probe")],
          env:{APP_CONFIG:lit("task override"),TASK_ONLY:lit("step overlay")}
        })],checks:["after"]}),
      ]});
    });
    ''')
    monkeypatch.setenv("UNDECLARED", "must not become wrapper authority")
    monkeypatch.setenv("SHELL", "/bin/sh")
    updated = grip("update", "tools", cwd=repo)
    assert updated.returncode == 0, updated.stdout + updated.stderr
    expected = "spaces '$HOME'||<unset>|<unset>|/reviewed prefix|42|192000"
    direct = grip("run", "--env", "dev", "--", "probe", "", "two words", "--flag", cwd=repo)
    assert direct.returncode == 0, direct.stdout + direct.stderr
    assert direct.stdout == expected + "<><two words><--flag>\n"
    nested = grip("run", "--env", "dev", "--", "/bin/sh", "-c",
                  'exec probe "" "two words" --flag', cwd=repo)
    assert nested.returncode == 0, nested.stdout + nested.stderr
    assert nested.stdout == direct.stdout
    shell = run_grip([str(GRIP.resolve()), "shell", "dev"], cwd=repo,
                     input='APP_CONFIG="shell override"; export APP_CONFIG; unset EMPTY; exec probe\n',
                     capture_output=True, text=True, timeout=120)
    assert shell.returncode == 0, shell.stdout + shell.stderr
    assert shell.stdout == "shell override|<unset>|<unset>|<unset>|/reviewed prefix|42|192000\n"
    task_result = grip("task", "nested", cwd=repo)
    assert task_result.returncode == 0, task_result.stdout + task_result.stderr
    assert task_result.stdout.splitlines() == [
        "task override||step overlay|<unset>|/reviewed prefix|42|192000",
        "spaces '$HOME'||check overlay|<unset>|/reviewed prefix|42|192000",
    ]
    applied = grip("apply", cwd=repo)
    assert applied.returncode == 0, applied.stdout + applied.stderr
    profile = sandbox / ".local/share/gripsack/current/env/profile.sh"
    projected = subprocess.run(
        ["/bin/sh", "-c", '. "$1"; exec probe "" "two words" --flag', "profile-test", str(profile)],
        capture_output=True, text=True, timeout=120,
    )
    assert projected.returncode == 0, projected.stdout + projected.stderr
    assert projected.stdout == direct.stdout
    stdin = subprocess.run(
        ["/bin/sh", "-c", '. "$1"; exec probe stdin', "profile-test", str(profile)],
        input="kept stdin\n", capture_output=True, text=True, timeout=120,
    )
    assert stdin.returncode == 0, stdin.stdout + stdin.stderr
    assert stdin.stdout == expected + "<stdin>|input=kept stdin\n"
    failed = subprocess.run(
        ["/bin/sh", "-c", '. "$1"; exec probe fail', "profile-test", str(profile)],
        capture_output=True, text=True, timeout=120,
    )
    assert failed.returncode == 7, failed.stdout + failed.stderr
    location = subprocess.run(
        ["/bin/sh", "-c", '. "$1"; command -v probe', "profile-test", str(profile)],
        capture_output=True, text=True, timeout=120, check=True,
    )
    wrapper = Path(location.stdout.strip())
    linked_wrapper = sandbox / "linked-wrapper"
    linked_wrapper.symlink_to(wrapper)
    linked = subprocess.run(
        ["/bin/sh", "-c", '. "$1"; exec "$2"', "profile-test", str(profile), str(linked_wrapper)],
        capture_output=True, text=True, timeout=120,
    )
    assert linked.returncode == 0, linked.stdout + linked.stderr
    assert linked.stdout == expected + "\n"
    tampered = sandbox / "tampered-wrapper"
    tampered.write_text(wrapper.read_text().replace('"APP_CONFIG"', '"UNDECLARED"', 1))
    tampered.chmod(0o755)
    refused = subprocess.run(
        ["/bin/sh", "-c", '. "$1"; exec "$2"', "profile-test", str(profile), str(tampered)],
        capture_output=True, text=True, timeout=120,
    )
    assert refused.returncode != 0
    assert refused.stdout == "", "a changed private environment binding must fail before the package executes"
