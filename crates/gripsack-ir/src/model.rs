use serde::{Deserialize, Serialize};

/// The current workspace wire document. Persisted generations and receipts
/// retain their separate versioned storage contracts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ir {
    pub ir_version: u32,
    pub host: HostFacts,
    pub workspace: crate::workspace_model::WorkspaceCatalog,
}

impl Ir {
    /// Missing capabilities are refused before CLI or direct executor effects.
    pub fn workspace_execution_error(
        &self,
        operation: crate::workspace::WorkspaceOperation,
    ) -> Option<crate::diagnostic::Diagnostic> {
        crate::workspace::execution_gate::execution_error(self, operation)
    }
}

/// Facts resolved at eval time; the core never re-derives them (0001 §5).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

/// Native acquisition sources. Plugin fetches carry transport-specific data.
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
        /// The registry's API asset endpoint (github releases) — the
        /// authenticated download path for private releases. Produced
        /// by resolution; frontends never set it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_url: Option<String>,
    },
    Git {
        url: String,
        /// Absent = float: the core pins the default branch's HEAD into
        /// the lockfile at resolve time (0016 §D2).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rev: Option<String>,
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
