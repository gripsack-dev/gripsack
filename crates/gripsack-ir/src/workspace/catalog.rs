//! Shared acquisition, execution and lifecycle domains.

use crate::{FetchSpec, Span};
use serde::{Deserialize, Serialize};

/// A native fetch declaration always carries its emitting source location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFetch {
    pub fetch: FetchSpec,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostAccess {
    /// No filesystem, kernel or network isolation claim.
    Unconfined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinuxWorker {
    Buildkit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeOutputKind {
    File,
    Tree,
}
