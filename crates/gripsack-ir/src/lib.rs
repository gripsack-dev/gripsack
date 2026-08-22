//! IR v1 (draft) — the contract between frontends and the core.
//! See plan/0001 §3.2, plan/0004 (spans, diagnostics, passes) and
//! schema/ir/v1.json. Change all three sides together
//! (`.agents/skills/gripsack-ir`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// The only IR version this core accepts (for now).
pub const IR_VERSION: u32 = 1;

/// Stable diagnostic codes (0004 §3). Match on codes, never on text.
pub mod codes {
    pub const MALFORMED: &str = "E000";
    pub const VERSION: &str = "E100";
    pub const UNKNOWN_DEPENDENCY: &str = "E101";
    pub const BAD_DESTINATION: &str = "E102";
    pub const STEPS_WITH_FIELDS: &str = "E103";
    pub const UNKNOWN_STEP: &str = "E104";
    pub const DUPLICATE_STEP: &str = "E106";
    pub const UNKNOWN_RESOURCE: &str = "W201";
}

/// Step ids every declarative module gets after expansion (0007 §2) —
/// the valid cross-module targets (`rust:install`) when the target
/// module does not declare explicit steps.
pub const SYNTHESIZED_STEP_IDS: [&str; 6] =
    ["fetch", "build", "install", "config", "activate", "done"];

/// The reserved barrier step id (0007 §2); explicit modules may not
/// claim it.
pub const BARRIER_STEP_ID: &str = "done";

/// Resources the core knows how to serialize/throttle (0007 §4).
/// Anything else warns (W201) — an open namespace.
pub const KNOWN_RESOURCES: [&str; 3] = ["network", "pixi-lock", "cargo-lock"];

/// Source location of an IR node in the user's frontend code (0004 §2).
/// Payload: threaded through passes, never recomputed, never part of
/// store-path identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub file: String,
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col: Option<u32>,
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.col {
            Some(col) => write!(f, "{}:{}:{}", self.file, self.line, col),
            None => write!(f, "{}:{}", self.file, self.line),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ir {
    pub ir_version: u32,
    #[serde(default)]
    pub host: HostFacts,
    pub modules: BTreeMap<String, Module>,
}

/// Facts resolved at eval time; the core never re-derives them (0001 §5).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HostFacts {
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Module {
    /// None for dotfiles-only modules — their content is their config
    /// files (0006 §2 level 1). Mutually exclusive with `steps`
    /// (0007 §1, E103).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    #[serde(default)]
    pub build: Build,
    #[serde(default)]
    pub install: Vec<Entry>,
    #[serde(default)]
    pub config: Vec<Entry>,
    #[serde(default)]
    pub depends: Vec<Dependency>,
    #[serde(default)]
    pub activate: Vec<Intent>,
    /// Explicit pipeline control (0007). Present means the declarative
    /// fields above must all be empty — the core rejects both-shapes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<Step>>,
    /// Where this module was declared (0004 §2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// How to obtain the module's payload. Plugin sources are opaque to the
/// core beyond `name` + `args` (0002 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    GithubRelease {
        repo: String,
        asset: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
        /// GitHub Enterprise etc. (0002 §2 rung 1).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
    },
    Tarball {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
    Git {
        url: String,
        rev: String,
    },
    File {
        path: String,
    },
    Plugin {
        name: String,
        #[serde(default)]
        args: serde_json::Value,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Build {
    #[default]
    None,
    CargoInstall,
    Make,
    /// Escape hatch — flagged, busts fine-grained caching (0001 §2).
    CustomShell {
        script: String,
    },
}

/// A store path mapped to a destination.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub mode: Ownership,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Dotfile ownership modes (0001 §3.7).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
    /// Store-owned symlink; edits go through the module.
    #[default]
    Owned,
    /// Copied from store; drift detected on next apply.
    TrackedCopy,
    /// Managed block merged into a foreign file.
    Merge,
    /// Rendered at activation from module variables.
    Template,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dependency {
    pub module: String,
    #[serde(default)]
    pub edge: EdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Build-only deps are ephemeral: present during build, referenced by no
/// generation, GC'd afterward (0001 §3.1).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    #[default]
    Runtime,
    Build,
}

/// Declared activation intent — translated by platform adapters, never
/// executed as a raw command (0001 §3.8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    #[serde(default)]
    pub trigger: Trigger,
    #[serde(flatten)]
    pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    PostLink,
    #[default]
    PostActivate,
    OnRemove,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Service {
        name: String,
        #[serde(default)]
        user: bool,
    },
    Fonts,
    DesktopEntry,
    /// Escape hatch — flagged, shown by `plan`.
    CustomShell {
        script: String,
    },
}

/// A step: one node in a module's execution DAG (0007 §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub action: StepAction,
    /// Sibling step ids, or cross-module `module:step` refs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    /// Named resources to acquire before running (0007 §4).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<String>,
    /// Reporting tag — never a scheduling barrier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepAction {
    Fetch {
        source: Source,
    },
    Build {
        spec: Build,
    },
    Install {
        entries: Vec<Entry>,
    },
    ConfigDeploy {
        entries: Vec<Entry>,
    },
    Intent {
        action: Box<Action>,
    },
    /// The honest escape hatch: declared, flagged, cache-busting.
    CustomShell {
        script: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Fetch,
    Build,
    Install,
    Config,
    Activate,
    Custom,
}

// ---------------------------------------------------------------- diagnostics

/// Compiler-style diagnostics (0004 §3): structured, span-labeled,
/// collected across passes. Rendered by the CLI; matched on `code` by
/// tooling and the future LSP.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub labels: Vec<Label>,
    pub help: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    /// None when the node has no span — the message carries context then.
    pub span: Option<Span>,
    pub note: String,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Diagnostic {
            code,
            severity: Severity::Error,
            message: message.into(),
            labels: Vec::new(),
            help: None,
        }
    }

    pub fn with_label(mut self, span: Option<Span>, note: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            note: note.into(),
        });
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}]: {}", self.severity, self.code, self.message)?;
        for label in &self.labels {
            match &label.span {
                Some(span) => write!(f, "\n  --> {span}")?,
                None => write!(f, "\n  = {}", label.note)?,
            }
            if !label.note.is_empty() && label.span.is_some() {
                write!(f, " — {}", label.note)?;
            }
        }
        if let Some(help) = &self.help {
            write!(f, "\n  help: {help}")?;
        }
        Ok(())
    }
}

/// Parse + validate in one call (the CLI's usual path). Collects
/// everything pass 2 finds — one bad module never hides another.
pub fn check(json: &str) -> Result<Ir, Vec<Diagnostic>> {
    let ir = parse(json).map_err(|d| vec![d])?;
    let diagnostics = validate(&ir);
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Err(diagnostics);
    }
    Ok(ir)
}

/// Pass 1 — parse (0004 §4): syntax + version gate.
pub fn parse(json: &str) -> Result<Ir, Diagnostic> {
    let ir: Ir = serde_json::from_str(json).map_err(|e| {
        Diagnostic::error(codes::MALFORMED, format!("invalid IR JSON: {e}"))
            .with_help("the frontend emitted malformed IR — this is a frontend bug")
    })?;
    if ir.ir_version != IR_VERSION {
        return Err(Diagnostic::error(
            codes::VERSION,
            format!(
                "unsupported ir_version {} (this core accepts {IR_VERSION})",
                ir.ir_version
            ),
        ));
    }
    Ok(ir)
}

/// Pass 2 — structural sema (0004 §4): collect all diagnostics.
pub fn validate(ir: &Ir) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    validate_steps(ir, &mut diagnostics);
    for (name, module) in &ir.modules {
        for dep in &module.depends {
            if !ir.modules.contains_key(&dep.module) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::UNKNOWN_DEPENDENCY,
                        format!("module {name:?} depends on unknown module {:?}", dep.module),
                    )
                    .with_label(
                        dep.span.clone().or_else(|| module.span.clone()),
                        "dependency declared here",
                    ),
                );
            }
        }
        for entry in module.install.iter().chain(module.config.iter()) {
            if !(entry.to.starts_with('/') || entry.to.starts_with("~/")) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::BAD_DESTINATION,
                        format!(
                            "module {name:?}: destination {:?} must be absolute or start with ~/",
                            entry.to
                        ),
                    )
                    .with_label(
                        entry.span.clone().or_else(|| module.span.clone()),
                        "entry declared here",
                    ),
                );
            }
        }
    }
    diagnostics
}

/// Steps pass (0007 §6): shape, refs, ids.
fn validate_steps(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    for (name, module) in &ir.modules {
        let Some(steps) = &module.steps else {
            continue;
        };
        // E103: explicit steps + any declarative field.
        let mixed = module.source.is_some()
            || module.build != Build::None
            || !module.install.is_empty()
            || !module.config.is_empty()
            || !module.activate.is_empty();
        if mixed {
            diagnostics.push(
                Diagnostic::error(
                    codes::STEPS_WITH_FIELDS,
                    format!(
                        "module {name:?} mixes `steps` with declarative fields \
                         (source/build/install/config/activate)"
                    ),
                )
                .with_label(module.span.clone(), "module declared here")
                .with_help("pick one shape: fields (expanded for you) or explicit steps"),
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for step in steps {
            // E106: duplicate or reserved ids.
            if !seen.insert(step.id.as_str()) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::DUPLICATE_STEP,
                        format!("module {name:?}: duplicate step id {:?}", step.id),
                    )
                    .with_label(step.span.clone().or_else(|| module.span.clone()), ""),
                );
            }
            if step.id == BARRIER_STEP_ID {
                diagnostics.push(
                    Diagnostic::error(
                        codes::DUPLICATE_STEP,
                        format!(
                            "module {name:?}: step id {BARRIER_STEP_ID:?} is reserved \
                             (the module's barrier step)"
                        ),
                    )
                    .with_label(step.span.clone().or_else(|| module.span.clone()), ""),
                );
            }
            // E104: unknown step refs.
            for need in &step.needs {
                let unknown = match need.split_once(':') {
                    Some((target_module, target_step)) => match ir.modules.get(target_module) {
                        None => true,
                        Some(target) => match &target.steps {
                            Some(target_steps) => !target_steps.iter().any(|s| s.id == target_step),
                            None => !SYNTHESIZED_STEP_IDS.contains(&target_step),
                        },
                    },
                    None => !steps.iter().any(|s| s.id == *need),
                };
                if unknown {
                    diagnostics.push(
                        Diagnostic::error(
                            codes::UNKNOWN_STEP,
                            format!(
                                "module {name:?}: step {:?} needs unknown step {need:?}",
                                step.id
                            ),
                        )
                        .with_label(step.span.clone().or_else(|| module.span.clone()), ""),
                    );
                }
            }
            // W201: unknown resource names.
            for resource in &step.resources {
                if !KNOWN_RESOURCES.contains(&resource.as_str()) {
                    diagnostics.push(Diagnostic {
                        code: codes::UNKNOWN_RESOURCE,
                        severity: Severity::Warning,
                        message: format!(
                            "module {name:?}: step {:?} requires unknown resource \
                             {resource:?} (no mutual exclusion will apply)",
                            step.id
                        ),
                        labels: vec![Label {
                            span: step.span.clone().or_else(|| module.span.clone()),
                            note: String::new(),
                        }],
                        help: Some(format!("known: {}", KNOWN_RESOURCES.join(", "))),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"{
        "ir_version": 1,
        "host": {"os": "linux", "arch": "x86_64", "tags": ["gui"]},
        "modules": {
            "helix": {
                "source": {"kind": "github_release", "repo": "helix-editor/helix",
                           "asset": "helix-{version}-x86_64-linux.tar.xz"},
                "install": [{"from": "bin/hx", "to": "~/.local/bin/hx", "mode": "owned"}],
                "config": [{"from": "config.toml", "to": "~/.config/helix/config.toml",
                            "mode": "tracked_copy"}],
                "depends": [{"module": "git", "edge": "runtime"}],
                "activate": [{"trigger": "post_activate",
                              "kind": "service", "name": "syncthing", "user": true}],
                "span": {"file": "modules/helix.py", "line": 4, "col": 1}
            },
            "git": {
                "source": {"kind": "tarball", "url": "https://example.invalid/git.tar.xz"}
            }
        }
    }"#;

    #[test]
    fn parses_and_validates_example() {
        let ir = check(EXAMPLE).unwrap();
        assert_eq!(ir.ir_version, 1);
        assert_eq!(ir.modules.len(), 2);
        let helix = &ir.modules["helix"];
        assert!(matches!(helix.source, Some(Source::GithubRelease { .. })));
        assert_eq!(helix.config[0].mode, Ownership::TrackedCopy);
        assert_eq!(helix.span.as_ref().unwrap().line, 4);
        let again = serde_json::to_string(&ir).unwrap();
        check(&again).unwrap();
    }

    #[test]
    fn unknown_dependency_carries_span_and_code() {
        let bad = EXAMPLE.replace(r#""module": "git""#, r#""module": "nope""#);
        let diagnostics = check(&bad).unwrap_err();
        assert_eq!(diagnostics.len(), 1);
        let d = &diagnostics[0];
        assert_eq!(d.code, codes::UNKNOWN_DEPENDENCY);
        let rendered = d.to_string();
        assert!(rendered.contains("error[E101]"));
        assert!(rendered.contains("modules/helix.py:4:1"));
    }

    #[test]
    fn collects_multiple_diagnostics() {
        let bad = EXAMPLE
            .replace(r#""module": "git""#, r#""module": "nope""#)
            .replace("~/.local/bin/hx", "bin/hx-elsewhere");
        let diagnostics = check(&bad).unwrap_err();
        assert_eq!(diagnostics.len(), 2);
        let codes: Vec<_> = diagnostics.iter().map(|d| d.code).collect();
        assert!(codes.contains(&codes::UNKNOWN_DEPENDENCY));
        assert!(codes.contains(&codes::BAD_DESTINATION));
    }

    #[test]
    fn rejects_wrong_version() {
        let bad = EXAMPLE.replace(r#""ir_version": 1"#, r#""ir_version": 99"#);
        let diagnostics = check(&bad).unwrap_err();
        assert_eq!(diagnostics[0].code, codes::VERSION);
    }

    #[test]
    fn malformed_json_is_e000() {
        let diagnostics = check("{not json").unwrap_err();
        assert_eq!(diagnostics[0].code, codes::MALFORMED);
        assert!(diagnostics[0].help.is_some());
    }

    #[test]
    fn ownership_and_edge_defaults() {
        let e: Entry = serde_json::from_str(r#"{"from":"a","to":"/b"}"#).unwrap();
        assert!(matches!(e.mode, Ownership::Owned));
        let d: Dependency = serde_json::from_str(r#"{"module":"m"}"#).unwrap();
        assert!(matches!(d.edge, EdgeKind::Runtime));
    }

    #[test]
    fn dotfiles_only_module_needs_no_source() {
        // 0006 §2 level 1: a module that only manages configs.
        let json = r#"{
            "ir_version": 1,
            "modules": {
                "helix": {
                    "config": [{"from": "config.toml",
                                "to": "~/.config/helix/config.toml",
                                "mode": "tracked_copy"}]
                }
            }
        }"#;
        let ir = check(json).unwrap();
        assert_eq!(ir.modules["helix"].source, None);
        assert_eq!(ir.modules["helix"].config.len(), 1);
    }

    const STEPPED: &str = r#"{
        "ir_version": 1,
        "modules": {
            "helix": {
                "steps": [
                    {"id": "fetch", "action": {"kind": "fetch",
                     "source": {"kind": "tarball", "url": "https://example.invalid/h.tar.xz"}},
                     "phase": "fetch"},
                    {"id": "patch", "action": {"kind": "custom_shell", "script": "true"},
                     "needs": ["fetch"], "phase": "custom"}
                ]
            },
            "rust": {
                "source": {"kind": "tarball", "url": "https://example.invalid/r.tar.xz"}
            }
        }
    }"#;

    #[test]
    fn explicit_steps_validate() {
        check(STEPPED).unwrap();
    }

    #[test]
    fn cross_module_ref_into_declarative_module() {
        let json = STEPPED.replace(
            r#""needs": ["fetch"]"#,
            r#""needs": ["fetch", "rust:install"]"#,
        );
        check(&json).unwrap();
        let bad = STEPPED.replace(r#""needs": ["fetch"]"#, r#""needs": ["fetch", "rust:wat"]"#);
        let diagnostics = check(&bad).unwrap_err();
        assert!(diagnostics.iter().any(|d| d.code == codes::UNKNOWN_STEP));
    }

    #[test]
    fn steps_plus_declarative_fields_is_e103() {
        let bad = STEPPED.replace(
            r#""steps": ["#,
            r#""source": {"kind": "file", "path": "/x"}, "steps": ["#,
        );
        let diagnostics = check(&bad).unwrap_err();
        assert_eq!(diagnostics[0].code, codes::STEPS_WITH_FIELDS);
        assert!(diagnostics[0].help.is_some());
    }

    #[test]
    fn duplicate_and_reserved_step_ids_are_e106() {
        let dup = STEPPED.replace(
            r#"{"id": "patch", "action": {"kind": "custom_shell", "script": "true"},
                     "needs": ["fetch"], "phase": "custom"}"#,
            r#"{"id": "fetch", "action": {"kind": "custom_shell", "script": "true"}}"#,
        );
        assert!(check(&dup)
            .unwrap_err()
            .iter()
            .any(|d| d.code == codes::DUPLICATE_STEP));

        let reserved = STEPPED.replace(r#""id": "patch""#, r#""id": "done""#);
        assert!(check(&reserved)
            .unwrap_err()
            .iter()
            .any(|d| d.code == codes::DUPLICATE_STEP));
    }

    #[test]
    fn unknown_resource_warns_but_passes() {
        let json = STEPPED.replace(
            r#""needs": ["fetch"]"#,
            r#""needs": ["fetch"], "resources": ["my-lock"]"#,
        );
        // warnings don't fail the check
        let ir = check(&json).unwrap();
        assert!(ir.modules["helix"].steps.is_some());
        let diagnostics = validate(&ir);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == codes::UNKNOWN_RESOURCE && d.severity == Severity::Warning));
    }

    #[test]
    fn spanless_nodes_still_render() {
        let d = Diagnostic::error(codes::UNKNOWN_DEPENDENCY, "no span here")
            .with_label(None, "module xyz");
        let rendered = d.to_string();
        assert!(rendered.contains("= module xyz"));
    }
}
