//! Runtime deployment primitives, independent of the workspace authoring contract.
//! Persisted enum spellings remain stable for retained generations and activation.

use serde::{Deserialize, Serialize};

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
