//! Runtime deployment primitives, independent of the workspace authoring contract.
//! Persisted enum spellings remain stable for retained generations and activation.

use gripsack_ir::Span;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An immutable payload mapped to a destination by profile preparation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentEntry {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub mode: Ownership,
    /// Retained template values used when replaying a deployment.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<String, String>,
    /// Managed-block comment prefix; absence infers it from the destination.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Destination ownership policy, including retained template receipts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
    /// Store-owned symlink; edits go through its profile declaration.
    #[default]
    Owned,
    /// Copied from store; drift detected on the next apply.
    TrackedCopy,
    /// Managed block merged into a foreign file.
    Merge,
    /// Rendered from retained template values during activation or rollback.
    Template,
}

/// How a retained contribution changes a shell-profile variable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentOperation {
    #[default]
    Set,
    Prepend,
    Append,
}

/// Lifecycle point at which a deployment's activation intent runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationTrigger {
    PostLink,
    #[default]
    PostActivate,
    OnRemove,
}
