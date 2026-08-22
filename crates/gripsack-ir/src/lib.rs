//! IR v1 (draft) — the contract between frontends and the core.
//! See plan/0001 §3.2 and schema/ir/v1.json. Change all three sides
//! together (`.agents/skills/gripsack-ir`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The only IR version this core accepts (for now).
pub const IR_VERSION: u32 = 1;

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
    pub source: Source,
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
    /// Where in the user's frontend code this module was declared.
    /// Preserved and surfaced, never interpreted (0001 §3.2).
    #[serde(default)]
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub file: String,
    pub line: u32,
}

/// How to obtain the module's payload. Plugin sources are opaque to the
/// core beyond `name` + `args` (0002 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    GithubRelease {
        repo: String,
        asset: String,
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        sha256: Option<String>,
        /// GitHub Enterprise etc. (0002 §2 rung 1).
        #[serde(default)]
        base_url: Option<String>,
    },
    Tarball {
        url: String,
        #[serde(default)]
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

#[derive(Debug, thiserror::Error)]
pub enum IrError {
    #[error("invalid IR JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported ir_version {0} (this core accepts {IR_VERSION})")]
    Version(u32),
    #[error("module {module:?} depends on unknown module {dep:?}")]
    UnknownDependency { module: String, dep: String },
    #[error("module {module:?}: destination {to:?} must be absolute or start with ~/")]
    BadDestination { module: String, to: String },
}

impl Ir {
    pub fn parse(json: &str) -> Result<Self, IrError> {
        let ir: Ir = serde_json::from_str(json)?;
        ir.validate()?;
        Ok(ir)
    }

    pub fn validate(&self) -> Result<(), IrError> {
        if self.ir_version != IR_VERSION {
            return Err(IrError::Version(self.ir_version));
        }
        for (name, module) in &self.modules {
            for dep in &module.depends {
                if !self.modules.contains_key(&dep.module) {
                    return Err(IrError::UnknownDependency {
                        module: name.clone(),
                        dep: dep.module.clone(),
                    });
                }
            }
            for entry in module.install.iter().chain(module.config.iter()) {
                if !(entry.to.starts_with('/') || entry.to.starts_with("~/")) {
                    return Err(IrError::BadDestination {
                        module: name.clone(),
                        to: entry.to.clone(),
                    });
                }
            }
        }
        Ok(())
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
                "provenance": {"file": "modules/helix.py", "line": 4}
            },
            "git": {
                "source": {"kind": "tarball", "url": "https://example.invalid/git.tar.xz"}
            }
        }
    }"#;

    #[test]
    fn parses_and_validates_example() {
        let ir = Ir::parse(EXAMPLE).unwrap();
        assert_eq!(ir.ir_version, 1);
        assert_eq!(ir.modules.len(), 2);
        let helix = &ir.modules["helix"];
        assert!(matches!(helix.source, Source::GithubRelease { .. }));
        assert_eq!(helix.config[0].mode, Ownership::TrackedCopy);
        assert_eq!(helix.provenance.as_ref().unwrap().line, 4);
        // round-trips losslessly enough to re-validate
        let again = serde_json::to_string(&ir).unwrap();
        Ir::parse(&again).unwrap();
    }

    #[test]
    fn rejects_unknown_dependency() {
        let bad = EXAMPLE.replace(r#""module": "git""#, r#""module": "nope""#);
        let err = Ir::parse(&bad).unwrap_err();
        assert!(matches!(err, IrError::UnknownDependency { dep, .. } if dep == "nope"));
    }

    #[test]
    fn rejects_relative_destination() {
        let bad = EXAMPLE.replace("~/.local/bin/hx", "bin/hx-elsewhere");
        assert!(matches!(
            Ir::parse(&bad),
            Err(IrError::BadDestination { .. })
        ));
    }

    #[test]
    fn rejects_wrong_version() {
        let bad = EXAMPLE.replace(r#""ir_version": 1"#, r#""ir_version": 99"#);
        assert!(matches!(Ir::parse(&bad), Err(IrError::Version(99))));
    }

    #[test]
    fn ownership_and_edge_defaults() {
        let e: Entry = serde_json::from_str(r#"{"from":"a","to":"/b"}"#).unwrap();
        assert!(matches!(e.mode, Ownership::Owned));
        let d: Dependency = serde_json::from_str(r#"{"module":"m"}"#).unwrap();
        assert!(matches!(d.edge, EdgeKind::Runtime));
    }
}
