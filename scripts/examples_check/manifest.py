"""Classification manifest: one entry per published TypeScript
example, paired by location only (file + window title / fence
ordinal). The CODE always comes from the docs at run time — this
manifest holds classification, expectations and fixture needs, never
example code.

kind:
  module   — default-exports a module() value; scaffolded under
             modules/ with a generated host entrypoint
  factory  — exports a factory (and, when the doc shows them, the
             built instances); a generated host lists the instances,
             so a factory that forgot `return module(...)` drops its
             modules from the IR and the expectation fails
  host     — IS a host entrypoint (defineEnv); placed verbatim
  workspace — root gripsack.ts (defineWorkspace); placed verbatim,
              without a synthetic host or module entrypoint
  fragment — illustrative snippet, not a runnable program; `reason`
             quotes the source evidence that makes it partial

expect_modules: module names that must appear in the emitted IR for
the default stage (tags [], probes unbound). A name gated on the
machine's os is ("name", "linux") — expectations follow the facts the
eval actually ran with, on any checker host.

expect_outputs: exact mapping of workspace output names to kinds.
Workspace `ir` points use ("output", name, path, expected), with paths
relative to the named output; module points retain ("module", ...).
Both forms assert emitted semantics, independently of declaration order.
"""

# Fixture module for examples that import modules the page does not
# show (steam/cuda/compiler): a plausible tool module over an offline
# payload — the harness supplies it rather than excluding the example.
FACTORY_FIXTURE_MODULE = (
    'import {{ fileFetch, module, symlink }} from "@gripsack/core";\n'
    "\n"
    'export default module("{name}", {{\n'
    '  fetch: fileFetch("payloads/{name}.tar.gz"),\n'
    '  install: {{ "bin/{bin}": symlink("~/.local/bin/{bin}") }},\n'
    "}});\n"
)

EXAMPLES = [
    {
        "id": "modules-helix-data-style",
        "file": "modules.md",
        "locator": {"window": "modules/helix.ts"},
        "kind": "module",
        "module_file": "helix.ts",
        "expect_modules": ["helix"],
        "ir": [
            ("module", "helix", "install[0]",
             {"from": "bin/hx", "to": "~/.local/bin/hx", "mode": "owned"}),
            ("module", "helix", "config[0]",
             {"from": "config.toml", "to": "~/.config/helix/config.toml",
              "mode": "tracked_copy"}),
            ("module", "helix", "fetch.kind", "github_release"),
        ],
    },
    {
        "id": "modules-explicit-steps",
        "file": "modules.md",
        "locator": {"window": "modules/patched.ts"},
        "kind": "module",
        "module_file": "patched.ts",
        "expect_modules": ["patched"],
        "fixtures": {
            "payloads/hello.tar.gz": {"payload": {"bin/hx": b"#!/bin/sh\n"}},
        },
        "ir": [
            ("module", "patched", "steps[0].action.kind", "fetch"),
            ("module", "patched", "steps[1].action.kind", "custom_shell"),
            ("module", "patched", "steps[1].action.script~", "patch -p1"),
            ("module", "patched", "steps[2].needs", ["patch"]),
        ],
    },
    {
        "id": "modules-factory-lang-servers",
        "file": "modules.md",
        "locator": {"window": "modules/lang-servers.ts"},
        "kind": "factory",
        "module_file": "lang-servers.ts",
        "exports": ["lua", "zed"],
        "expect_modules": ["lua-ls", "zed"],
        "ir": [
            ("module", "lua-ls", "install[0].to", "~/.local/bin/lua-ls"),
            ("module", "zed", "install[0].to", "~/.local/bin/zed"),
        ],
    },
    {
        "id": "modules-host-laptop",
        "file": "modules.md",
        "locator": {"window": "hosts/laptop.ts"},
        "kind": "host",
        "host": "laptop",
        # the example imports modules the page does not show — the
        # harness provides them (complete example, fixtures supplied,
        # not excluded)
        "fixtures": {
            "modules/steam.ts": FACTORY_FIXTURE_MODULE.format(
                name="steam", bin="steam"),
            "modules/cuda.ts": FACTORY_FIXTURE_MODULE.format(
                name="cuda", bin="cuda"),
            "payloads/steam.tar.gz": {"payload": {"bin/steam": b"#!/bin/sh\n"}},
            "payloads/cuda.tar.gz": {"payload": {"bin/cuda": b"#!/bin/sh\n"}},
        },
        # os-gated steam stays, probe-gated cuda needs a bound answer
        "expect_modules": [("steam", "linux")],
        "probe_requests": [["executable", "nvidia-smi"]],
        "expect_modules_bound": [("steam", "linux"), "cuda"],
    },
    {
        "id": "modules-host-zed-conditional",
        "file": "modules.md",
        "locator": {"window": "hosts/laptop.ts — per-file conditionals"},
        "kind": "host",
        "host": "laptop",
        "fixtures": {
            "settings.laptop.json": '{"fixture": "laptop"}\n',
            "settings.spaces.json": '{"fixture": "spaces"}\n',
        },
        "expect_modules": ["zed"],
        # the per-file conditional must actually flip the chosen source:
        # tags [] → settings.laptop.json, tags ["spaces"] → the other
        "tag_stages": [
            {"tags": [], "point": ("module", "zed", "config[0].from",
                                   "settings.laptop.json")},
            {"tags": ["spaces"], "point": ("module", "zed", "config[0].from",
                                           "settings.spaces.json")},
        ],
    },
    {
        "id": "modules-build-closure",
        "file": "modules.md",
        "locator": {"fence": 0},
        "kind": "module",
        "module_file": "consumer.ts",
        # "Both modules must be listed in the host environment" — the
        # compiler the page references is provided as a fixture
        "fixtures": {
            "modules/compiler.ts": FACTORY_FIXTURE_MODULE.format(
                name="compiler", bin="cc"),
            "payloads/compiler.tar.gz": {"payload": {"bin/cc": b"#!/bin/sh\n"}},
        },
        "expect_modules": ["consumer", "compiler"],
        # the compiler fixture must be LISTED too — registration is
        # the host entrypoint's job, an unimported file is inert
        "also_modules": ["compiler"],
        "ir": [
            ("module", "consumer", "depends[0]",
             {"module": "compiler", "for": "build"}),
            ("module", "consumer", "steps[1].needs", ["build"]),
        ],
    },
    {
        "id": "fetchers-apt-jq",
        "file": "fetchers/apt.md",
        "locator": {"window": "a module fetching from apt"},
        "kind": "module",
        "module_file": "jq.ts",
        # runnable as-is at check time: the plugin fetcher resolves
        # at apply, not eval — no fixture, no network
        "expect_modules": ["jq"],
        "ir": [
            ("module", "jq", "fetch.kind", "plugin"),
            ("module", "jq", "fetch.args.package", "jq"),
            ("module", "jq", "verify.kind", "binary_runs"),
            ("module", "jq", "verify.args[0]", "--version"),
        ],
    },
    {
        "id": "changelog-factory-langserver",
        "file": "changelog.md",
        "locator": {"fence": 0},
        "kind": "factory",
        "module_file": "lang-server.ts",
        # recorded scaffold adaptations (changelog prose strips the
        # module-file context): re-add the import preamble and the
        # `export` keyword — the body itself comes from the doc
        "preamble": 'import { githubRelease, module, symlink } from "@gripsack/core";\n\n',
        "export_fixup": True,
        "factory_call": ["langServer", ["lua-ls", "LuaLS/lua-language-server"]],
        "expect_modules": ["lua-ls"],
    },
    {
        "id": "linters-yazi-optin",
        "file": "linters.md",
        "locator": {"fence": 0},
        "kind": "fragment",
        "reason": (
            "the module spec is elided in the source — "
            '`export default module("yazi", { /* \u2026 */ lint: "yazi" });` '
            "is an opt-in illustration, not a runnable program (a completed "
            "variant would also need the [linters.yazi] registration the "
            "page shows above it)"
        ),
    },
    {
        "id": "environments-workspace-node",
        "file": "environments.md",
        "locator": {"window": "gripsack.ts"},
        "kind": "workspace",
        "expect_outputs": {
            "node": "package",
            "tools": "environment",
            "with-node": "profile",
        },
        "ir": [
            ("output", "node", "producer.kind", "provider"),
            ("output", "node", "producer.provider.kind", "conda_environment"),
            ("output", "node", "producer.provider.channels", ["conda-forge"]),
            ("output", "node", "producer.provider.packages", {"nodejs": "=26.10.0"}),
            ("output", "node", "producer.provider.system_requirements.libc",
             {"family": "glibc", "version": "2.28"}),
            ("output", "node", "producer.provider.system_requirements.linux", "4.18"),
            ("output", "node", "commands", {"node": "bin/node"}),
            ("output", "node", "target", {"os": "linux", "arch": "x86_64", "abi": "gnu"}),
            ("output", "node", "layout.kind", "prefix_materialized"),
            ("output", "tools", "packages", ["node"]),
            ("output", "tools", "target", {"os": "linux", "arch": "x86_64", "abi": "gnu"}),
            ("output", "with-node", "environment", "tools"),
        ],
    },
    {
        "id": "environments-managed-launcher",
        "file": "environments.md",
        "locator": {"window": "gripsack.ts — managed launcher"},
        "kind": "workspace",
        "fixtures": {
            "launchers/gripsack-node": {
                "executable": (
                    "#!/bin/sh\nset -eu\n"
                    '. "${GRIPSACK_HOME:?}/current/env/profile.sh"\n'
                    'exec node "$@"\n'
                ),
            },
        },
        "expect_outputs": {
            "node": "package",
            "tools": "environment",
            "with-node": "profile",
        },
        "ir": [
            ("output", "node", "producer.provider.system_requirements.libc",
             {"family": "glibc", "version": "2.28"}),
            ("output", "with-node", "environment", "tools"),
            ("output", "with-node", "files[0].source",
             {"kind": "repo_file", "path": "launchers/gripsack-node"}),
            ("output", "with-node", "files[0].content.kind", "identity"),
            ("output", "with-node", "files[0].destination",
             {"kind": "symlink", "path": "~/.local/bin/gripsack-node"}),
        ],
    },
    {
        "id": "environments-physical-release-layout",
        "file": "environments.md",
        "locator": {"window": "gripsack.ts — physical release layout"},
        "kind": "workspace",
        "fixtures": {
            "release/bin/sibling-tool": {
                "executable": (
                    "#!/bin/sh\nset -eu\n"
                    'self=$(readlink -f "$0")\n'
                    'base=$(dirname "$self")\n'
                    'cat "$base/../lib/message"\n'
                ),
            },
            "release/lib/message": "physical-siblings-ok\n",
        },
        "expect_outputs": {"release": "package", "release-files": "profile"},
        "ir": [
            ("output", "release", "producer.provider.fetch",
             {"kind": "file", "path": "release"}),
            ("output", "release-files", "files[0].source",
             {"kind": "tree", "output": "release", "include": ["bin", "lib"]}),
            ("output", "release-files", "files[0].content.kind", "identity"),
            ("output", "release-files", "files[0].destination",
             {"kind": "tracked_copy", "path": "~/.local/lib/sibling-release"}),
        ],
    },
    {
        "id": "environments-profile-hook",
        "file": "environments.md",
        "locator": {"window": "gripsack.ts — a profile hook"},
        "kind": "fragment",
        "reason": (
            "hook declaration is real but elided from its selecting profile "
            "— `profile(…, { hooks: [\"after\"] })` appears in prose only"
        ),
    },
    {
        "id": "migration-conda-environment",
        "file": "workspace-migration.md",
        "locator": {"window": "gripsack.ts — conda.environment"},
        "kind": "fragment",
        "reason": (
            "pkg(...) fragment with the environment/profile continuation "
            "elided to comments — the complete shape lives on environments.md"
        ),
    },
    {
        "id": "migration-pixi-fromlock",
        "file": "workspace-migration.md",
        "locator": {"window": "gripsack.ts — pixi.fromLock"},
        "kind": "fragment",
        "reason": (
            "input + provider declarations only; the workspace({inputs, "
            "outputs}) wiring is elided in a trailing comment"
        ),
    },
]
