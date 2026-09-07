//! Schema ↔ parser acceptance parity (plan/0042 §E): every corpus
//! document is admitted or rejected by BOTH `schema/ir/v3.json` and
//! the real core admission (`parse` + the tagged-field pass) — never
//! by guessed serde behavior. Semantics (graphs, paths, phases) stay
//! in sema and are deliberately absent here.
//!
//! The schema is checked with the real `jsonschema` crate (dev-only,
//! default-features off — no HTTP stack; the IR schema has no remote
//! refs), compiled once per test. `pattern` is an assertion: the
//! `schema_pattern_assertions_execute` test proves it rejects.
//!
//! Known, documented asymmetries (excluded from the parity corpus):
//! - JSON Schema numeric equality is mathematical, so `3.0` satisfies
//!   `const: 3` while serde rejects it for `ir_version: u32`.
//! - Authored-value constraints (sha256 hex, destination prefix, repo
//!   shape, minLength) are deliberately stricter than the parser —
//!   they constrain what a frontend may author. They are pinned in
//!   their own test, not the parity corpus.
//! - `format` stays an annotation (draft 2020-12 default), matching
//!   the crate's default behavior.
//! - `plugin.args`: the parser admits any JSON value; the authored
//!   contract (and TS emitter) is an object, or absent/null.

use gripsack_ir::parse;
use jsonschema::validator_for;
use serde_json::{Value, json};

/// The canonical schema, reached through the tracked symlink
/// `crates/gripsack-ir/schema-v3.json → ../../schema/ir/v3.json` so
/// the packaged crate carries it without a maintained shadow copy.
const SCHEMA: &str = include_str!("../schema-v3.json");

/// Compile the schema once; each test validates its whole corpus
/// against the returned validator.
fn compiled() -> jsonschema::Validator {
    let parsed: Value = serde_json::from_str(SCHEMA).expect("schema is valid JSON");
    validator_for(&parsed).expect("schema compiles")
}

/// None = the schema admits the document.
fn schema_error(validator: &jsonschema::Validator, doc: &Value) -> Option<String> {
    validator.validate(doc).err().map(|e| e.to_string())
}

// ------------------------------------------------------------------ corpus

fn doc(module: Value) -> Value {
    json!({"ir_version": 3, "modules": {"m": module}})
}

fn step(action: Value) -> Value {
    json!({"id": "s", "action": action})
}

/// Maximal shapes: every fetch kind's optionals exercised as explicit
/// null, every entry/intent/verify/step-action variant, u32 span
/// bounds, and an explicit-null sweep over Option fields.
fn kitchen_sink() -> Value {
    json!({
        "ir_version": 3,
        "host": {"os": "linux", "arch": "x86_64", "tags": ["gui"], "libc": "glibc-2.36"},
        "resources": [{"name": "company.lock"}],
        "modules": {
            "helix": {
                "fetch": {"kind": "github_release", "repo": "helix-editor/helix",
                          "asset": "hx-{version}.tar.xz", "version": null,
                          "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                          "base_url": null},
                "build": {"kind": "custom_shell", "script": "make install"},
                "install": [{"from": "bin/hx", "to": "~/.local/bin/hx", "mode": "owned"}],
                "config": [
                    {"from": "config.toml", "to": "~/.config/helix/config.toml",
                     "mode": "tracked_copy"},
                    {"from": "launch.jsonc", "to": "~/.config/helix/launch.jsonc",
                     "mode": "merge", "marker": "//", "span": null},
                    {"from": "theme", "to": "~/.config/helix/theme.toml", "mode": "template",
                     "vars": {"accent": "cyan"},
                     "span": {"file": "modules/helix.ts", "line": 12, "col": null}}
                ],
                "depends": [
                    {"module": "git", "for": "runtime",
                     "span": {"file": "modules/helix.ts", "line": 0, "col": 4294967295u32}},
                    {"module": "rust", "for": "build", "span": null}
                ],
                "activate": [
                    {"kind": "service", "name": "hx-lsp", "user": false,
                     "trigger": "post_activate", "span": null},
                    {"kind": "fonts", "trigger": "post_link"},
                    {"kind": "desktop_entry", "trigger": "post_link"},
                    {"kind": "custom_shell", "script": "true", "trigger": "on_remove"}
                ],
                "env": [
                    {"name": "EDITOR", "op": "set", "value": "hx"},
                    {"name": "PATH", "op": "prepend", "value": "{store}/bin"},
                    {"name": "MANPATH", "op": "append", "value": "{store}/man"}
                ],
                "verify": {"kind": "binary_runs", "path": "bin/hx", "args": ["--version"]},
                "lint": "helix",
                "span": {"file": "modules/helix.ts", "line": 4}
            },
            "plumbed": {
                "fetch": {"kind": "plugin", "name": "internal",
                          "args": {"channel": "stable"}},
                "span": null
            },
            "stepped": {
                "steps": [
                    {"id": "fetch", "action": {"kind": "fetch",
                        "fetch": {"kind": "git", "url": "https://example.invalid/z", "rev": null}},
                     "phase": "fetch"},
                    {"id": "build", "action": {"kind": "build",
                        "spec": {"kind": "custom_shell", "script": "make"}},
                     "needs": ["fetch"], "phase": "build"},
                    {"id": "install", "action": {"kind": "install",
                        "entries": [{"from": "out/z", "to": "~/.local/bin/z", "mode": "owned"}]},
                     "needs": ["build"]},
                    {"id": "config", "action": {"kind": "config_deploy",
                        "entries": [{"from": "z.toml", "to": "~/.config/z/z.toml",
                                     "mode": "tracked_copy"}]},
                     "needs": ["install"], "verify": null},
                    {"id": "docs", "action": {"kind": "run", "argv": ["make", "docs"],
                        "env": {"JOBS": "4"}, "cwd": null, "outputs": ["docs/index.html"]},
                     "needs": ["build"], "resources": ["company.lock"], "phase": "custom",
                     "span": null},
                    {"id": "patch", "action": {"kind": "custom_shell", "script": "true",
                        "outputs": ["patched"]},
                     "needs": ["fetch"], "phase": "custom"},
                    {"id": "check", "action": {"kind": "verify",
                        "verify": {"kind": "file_deployed", "path": "~/.config/z/z.toml"}},
                     "needs": ["config"], "phase": "verify"},
                    {"id": "hook", "action": {"kind": "intent",
                        "action": {"kind": "service", "name": "zd", "user": true},
                        "trigger": "on_remove"},
                     "phase": "activate"}
                ],
                "verify": null,
                "span": {"file": "modules/stepped.ts", "line": 9}
            },
            "bottled": {"fetch": {"kind": "brew", "formula": "ripgrep",
                                  "version": null, "sha256": null}},
            "pixied": {"fetch": {"kind": "pixi", "package": "ripgrep",
                                 "version": null, "sha256": null}},
            "tarred": {"fetch": {"kind": "tarball",
                                 "url": "https://example.invalid/t.tar.zst",
                                 "sha256": null, "api_url": null}},
            "local": {"fetch": {"kind": "file", "path": "/opt/payload"}},
            "bare-plugin": {"fetch": {"kind": "plugin", "name": "x", "args": null}},
            "dotfiles": {"config": [{"from": "git.conf", "to": "~/.gitconfig",
                                     "mode": "tracked_copy"}]}
        }
    })
}

fn valid_corpus() -> Vec<(&'static str, Value)> {
    vec![
        (
            "kitchen sink (every variant, null optionals, u32 bounds)",
            kitchen_sink(),
        ),
        (
            "host absent and empty modules map",
            json!({"ir_version": 3, "modules": {}}),
        ),
        ("minimal module", doc(json!({"lint": null}))),
        (
            "line 0 and max u32 span",
            doc(json!({
                "fetch": {"kind": "file", "path": "p"},
                "span": {"file": "m.ts", "line": 0, "col": 4294967295u32}
            })),
        ),
        (
            "steps null on a declarative module",
            doc(json!({
                "fetch": {"kind": "file", "path": "p"}, "steps": null
            })),
        ),
        (
            "explicit nulls on optional module and step fields",
            doc(json!({
                "fetch": null,
                "steps": [
                    {"id": "s", "phase": null, "action": {"kind": "run", "argv": ["a"]}}
                ]
            })),
        ),
    ]
}

#[test]
fn valid_documents_are_admitted_by_both_sides() {
    let validator = compiled();
    for (name, document) in valid_corpus() {
        let text = document.to_string();
        let ir = parse(&text).unwrap_or_else(|e| panic!("{name}: parse rejected: {e}"));
        assert!(
            schema_error(&validator, &document).is_none(),
            "{name}: schema rejected valid doc: {:?}",
            schema_error(&validator, &document)
        );
        // serialization must stay inside the schema (skip_serializing_if
        // and enum spellings are part of the contract)
        let round: Value = serde_json::from_str(&serde_json::to_string(&ir).unwrap()).unwrap();
        assert!(
            schema_error(&validator, &round).is_none(),
            "{name}: serialized IR left the schema: {:?}",
            schema_error(&validator, &round)
        );
    }
}

#[test]
fn rejected_documents_fail_both_sides() {
    let validator = compiled();
    let cases: Vec<(&'static str, Value)> = vec![
        (
            "unknown top-level field",
            json!({"ir_version": 3, "modules": {}, "moduels": {}}),
        ),
        ("missing modules", json!({"ir_version": 3})),
        (
            "modules is an array",
            json!({"ir_version": 3, "modules": []}),
        ),
        ("ir_version 2", json!({"ir_version": 2, "modules": {}})),
        ("ir_version missing", json!({"modules": {}})),
        (
            "ir_version string",
            json!({"ir_version": "3", "modules": {}}),
        ),
        (
            "host is null",
            json!({"ir_version": 3, "host": null, "modules": {}}),
        ),
        (
            "host unknown field",
            json!({"ir_version": 3, "host": {"osk": "linux"}, "modules": {}}),
        ),
        (
            "resources is null",
            json!({"ir_version": 3, "resources": null, "modules": {}}),
        ),
        (
            "resource unknown field",
            json!({"ir_version": 3, "resources": [{"name": "r", "nme": "r"}], "modules": {}}),
        ),
        // span (u32 bounds)
        (
            "span unknown field",
            doc(json!({"span": {"file": "m", "line": 1, "lne": 2}})),
        ),
        ("span missing line", doc(json!({"span": {"file": "m"}}))),
        (
            "span file null",
            doc(json!({"span": {"file": null, "line": 1}})),
        ),
        (
            "span line negative",
            doc(json!({"span": {"file": "m", "line": -1}})),
        ),
        (
            "span line past u32",
            doc(json!({"span": {"file": "m", "line": 4294967296u64}})),
        ),
        (
            "span line fractional",
            doc(json!({"span": {"file": "m", "line": 1.5}})),
        ),
        (
            "span line string",
            doc(json!({"span": {"file": "m", "line": "3"}})),
        ),
        (
            "span col boolean",
            doc(json!({"span": {"file": "m", "line": 1, "col": true}})),
        ),
        // module shape
        ("module unknown field", doc(json!({"confg": []}))),
        ("build is null", doc(json!({"build": null}))),
        (
            "build missing kind",
            doc(json!({"build": {"script": "true"}})),
        ),
        ("build unknown kind", doc(json!({"build": {"kind": "nix"}}))),
        (
            "build none with script",
            doc(json!({"build": {"kind": "none", "script": "true"}})),
        ),
        (
            "build custom_shell missing script",
            doc(json!({"build": {"kind": "custom_shell"}})),
        ),
        ("steps is an object", doc(json!({"steps": {"id": "s"}}))),
        ("lint is a number", doc(json!({"lint": 3}))),
        // fetch
        (
            "fetch unknown kind",
            doc(json!({"fetch": {"kind": "http", "url": "u"}})),
        ),
        (
            "github fetch missing repo",
            doc(json!({"fetch": {"kind": "github_release", "asset": "a"}})),
        ),
        (
            "github fetch camelCase leak",
            doc(json!({"fetch": {"kind": "github_release", "repo": "a/b",
                                 "asset": "a", "baseUrl": "https://ghe"}})),
        ),
        (
            "tarball url null",
            doc(json!({"fetch": {"kind": "tarball", "url": null}})),
        ),
        (
            "git unknown field",
            doc(json!({"fetch": {"kind": "git", "url": "u", "branch": "main"}})),
        ),
        (
            "plugin missing name",
            doc(json!({"fetch": {"kind": "plugin"}})),
        ),
        (
            "brew unknown field",
            doc(json!({"fetch": {"kind": "brew", "formula": "f", "cask": "c"}})),
        ),
        (
            "pixi package number",
            doc(json!({"fetch": {"kind": "pixi", "package": 3}})),
        ),
        // entries
        (
            "entry unknown field",
            doc(json!({"install": [{"from": "a", "to": "~/b", "modes": "owned"}]})),
        ),
        (
            "entry mode null",
            doc(json!({"install": [{"from": "a", "to": "~/b", "mode": null}]})),
        ),
        (
            "entry unknown mode",
            doc(json!({"install": [{"from": "a", "to": "~/b", "mode": "symlink"}]})),
        ),
        (
            "entry vars null",
            doc(json!({"install": [{"from": "a", "to": "~/b", "vars": null}]})),
        ),
        (
            "entry to null",
            doc(json!({"install": [{"from": "a", "to": null}]})),
        ),
        ("entry missing to", doc(json!({"install": [{"from": "a"}]}))),
        // dependencies
        (
            "dependency unknown field",
            doc(json!({"depends": [{"module": "a", "edge": "runtime"}]})),
        ),
        (
            "dependency unknown purpose",
            doc(json!({"depends": [{"module": "a", "for": "buidl"}]})),
        ),
        (
            "dependency module null",
            doc(json!({"depends": [{"module": null}]})),
        ),
        // intents
        (
            "intent unknown field",
            doc(json!({"activate": [{"kind": "service", "name": "s",
                                     "user": true, "userr": true}]})),
        ),
        (
            "fonts intent with script",
            doc(json!({"activate": [{"kind": "fonts", "script": "true"}]})),
        ),
        (
            "service intent missing name",
            doc(json!({"activate": [{"kind": "service", "user": true}]})),
        ),
        (
            "intent unknown kind",
            doc(json!({"activate": [{"kind": "shortcut"}]})),
        ),
        (
            "intent trigger number",
            doc(json!({"activate": [{"kind": "fonts", "trigger": 5}]})),
        ),
        // verify
        (
            "verify unknown field",
            doc(json!({"verify": {"kind": "file_exists", "path": "p", "args": []}})),
        ),
        (
            "verify unknown kind",
            doc(json!({"verify": {"kind": "pytest"}})),
        ),
        (
            "binary_runs missing path",
            doc(json!({"verify": {"kind": "binary_runs"}})),
        ),
        (
            "binary_runs args null",
            doc(json!({"verify": {"kind": "binary_runs", "path": "p", "args": null}})),
        ),
        (
            "shell missing script",
            doc(json!({"verify": {"kind": "shell"}})),
        ),
        // env
        (
            "env name null",
            doc(json!({"env": [{"name": null, "value": "v"}]})),
        ),
        (
            "env unknown op",
            doc(json!({"env": [{"name": "A", "op": "seti", "value": "v"}]})),
        ),
        (
            "env op null",
            doc(json!({"env": [{"name": "A", "op": null, "value": "v"}]})),
        ),
        (
            "env value number",
            doc(json!({"env": [{"name": "A", "value": 3}]})),
        ),
        (
            "env unknown field",
            doc(json!({"env": [{"name": "A", "value": "v", "scope": "user"}]})),
        ),
        // steps
        (
            "step unknown field",
            doc(
                json!({"steps": [{"id": "s", "action": {"kind": "run", "argv": ["a"]},
                                  "need": ["s"]}]}),
            ),
        ),
        (
            "step missing id",
            doc(json!({"steps": [{"action": {"kind": "run", "argv": ["a"]}}]})),
        ),
        (
            "step needs null",
            doc(json!({"steps": [{"id": "s", "needs": null,
                                  "action": {"kind": "run", "argv": ["a"]}}]})),
        ),
        (
            "action unknown kind",
            doc(json!({"steps": [step(json!({"kind": "deploy", "to": "~"}))]})),
        ),
        (
            "run missing argv",
            doc(json!({"steps": [step(json!({"kind": "run"}))]})),
        ),
        (
            "run cwd number",
            doc(json!({"steps": [step(json!({"kind": "run", "argv": ["a"], "cwd": 5}))]})),
        ),
        (
            "run env null",
            doc(json!({"steps": [step(json!({"kind": "run", "argv": ["a"], "env": null}))]})),
        ),
        (
            "intent action missing",
            doc(json!({"steps": [step(json!({"kind": "intent", "trigger": "post_link"}))]})),
        ),
        (
            "intent step action unknown field",
            doc(json!({"steps": [step(json!({"kind": "intent",
                "action": {"kind": "fonts", "nme": "f"}}))]})),
        ),
        (
            "step fetch spec leaks a field",
            doc(json!({"steps": [step(json!({"kind": "fetch",
                "fetch": {"kind": "git", "url": "u", "depth": 1}}))]})),
        ),
        (
            "step build spec none-with-script",
            doc(json!({"steps": [step(json!({"kind": "build",
                "spec": {"kind": "none", "script": "true"}}))]})),
        ),
        (
            "step verify check leaks a field",
            doc(json!({"steps": [step(json!({"kind": "verify",
                "verify": {"kind": "shell", "script": "true", "timeout": 5}}))]})),
        ),
        (
            "step-level verify leaks a field",
            doc(json!({"steps": [{"id": "s", "verify": {"kind": "shell",
                                  "script": "true", "timeout": 5},
                                  "action": {"kind": "run", "argv": ["a"]}}]})),
        ),
        (
            "install action missing entries",
            doc(json!({"steps": [step(json!({"kind": "install"}))]})),
        ),
        (
            "unknown phase",
            doc(json!({"steps": [{"id": "s", "phase": "deploy",
                                  "action": {"kind": "run", "argv": ["a"]}}]})),
        ),
    ];
    for (name, document) in cases {
        let text = document.to_string();
        assert!(
            parse(&text).is_err(),
            "{name}: parser admitted a rejected doc"
        );
        assert!(
            schema_error(&validator, &document).is_some(),
            "{name}: schema admitted a rejected doc"
        );
    }
}

/// The schema's authored-value constraints (pattern, minLength) are
/// real assertions, deliberately stricter than the structural parser:
/// the parser accepts these documents, the schema rejects them. Each
/// row names the frontend-authoring rule it pins.
#[test]
fn schema_pattern_assertions_execute() {
    let validator = compiled();
    let cases: Vec<(&'static str, Value)> = vec![
        (
            "sha256 must be 64 lowercase hex",
            doc(
                json!({"fetch": {"kind": "github_release", "repo": "a/b", "asset": "a",
                                 "sha256": "nothex"}}),
            ),
        ),
        (
            "sha256 rejects uppercase hex",
            doc(json!({"fetch": {"kind": "brew", "formula": "f",
                                 "sha256": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}})),
        ),
        (
            "destination must be absolute or ~/-prefixed",
            doc(json!({"install": [{"from": "a", "to": "relative/b"}]})),
        ),
        (
            "github repo must be owner/name",
            doc(
                json!({"fetch": {"kind": "github_release", "repo": "noslash",
                                 "asset": "a"}}),
            ),
        ),
        (
            "dependency module name must be non-empty",
            doc(json!({"depends": [{"module": ""}]})),
        ),
        (
            "step id must be non-empty",
            doc(json!({"steps": [{"id": "", "action": {"kind": "run", "argv": ["a"]}}]})),
        ),
    ];
    for (name, document) in cases {
        let text = document.to_string();
        assert!(
            parse(&text).is_ok(),
            "{name}: parser rejected — not an authored-value boundary"
        );
        assert!(
            schema_error(&validator, &document).is_some(),
            "{name}: pattern/minLength did not reject"
        );
    }
}
