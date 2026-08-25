use crate::span::Span;
use crate::step::Step;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ir {
    pub ir_version: u32,
    #[serde(default)]
    pub host: HostFacts,
    /// Declared resources (0007 §4): step `resources` must resolve to
    /// these or the core's built-ins, else E107.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<Resource>,
    pub modules: BTreeMap<String, Module>,
}

/// A named, declared resource — a marker closing the namespace so typos
/// are sema errors, not silent "no mutual exclusion".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    pub name: String,
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
    /// e.g. "glibc-2.36", "musl", "darwin" — matters for binary asset
    /// selection in platform-conditional fetches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub libc: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Module {
    /// None for dotfiles-only modules — their content is their config
    /// files (0006 §2 level 1). Mutually exclusive with `steps`
    /// (0007 §1, E103).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch: Option<FetchSpec>,
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
    /// Environment contributions exported to the shell profile at
    /// activation (0001 §3.10). `{store}` in the value resolves to the
    /// module's store path.
    #[serde(default)]
    pub env: Vec<EnvVar>,
    /// Explicit pipeline control (0007). Present means the declarative
    /// fields above must all be empty — the core rejects both-shapes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<Step>>,
    /// Module-level smoke contract — a synthesized terminal step in the
    /// pipeline, run pre-flip (0007 §verify).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<Verify>,
    /// Retry default for this module's steps (0007 §retries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<u32>,
    /// Where this module was declared (0004 §2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// How to obtain the module's payload — a fetch spec. Plugin fetches are
/// opaque to the core beyond `name` + `args` (0002 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FetchSpec {
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
    /// Homebrew bottle : resolved from the formula JSON — bottle
    /// sha256 included, so pinning needs no download.
    Brew {
        formula: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
    /// Conda package via pixi : installed into an isolated
    /// PIXI_HOME and harvested into the store.
    Pixi {
        package: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
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
    /// Template variables (`{{ name }}` in the payload) — mode `template`
    /// only. Values are computed by the frontend at eval time (facts,
    /// per-host selection); the core stays a dumb substituter.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: std::collections::BTreeMap<String, String>,
    /// Comment prefix for the managed block (mode `merge` only), e.g.
    /// `//` for a jsonc dest. None → inferred from the dest extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
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

/// How a variable reaches the profile: set outright, or grow a
/// list-style variable (PATH & friends) at either end.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvOp {
    #[default]
    Set,
    Prepend,
    Append,
}

/// One environment contribution (0001 §3.10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    #[serde(default)]
    pub op: EnvOp,
    pub value: String,
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

/// A verify check — a smoke contract, not a test framework (0007 §verify).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Verify {
    BinaryRuns {
        path: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
    },
    FileExists {
        path: String,
    },
    Shell {
        script: String,
    },
    /// Check a deployed *destination* (not the payload) — for
    /// config-only modules (0009 critique: verify_file can't).
    FileDeployed {
        path: String,
    },
}
