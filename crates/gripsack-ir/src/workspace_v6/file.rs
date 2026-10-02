//! v6 profile file declarations (0052 §2.2 FileDecl, v6-design §2.3):
//! origin, content and destination policy stay three orthogonal axes.
//! v6 adds the `tree` origin (artifact subtree with include/exclude
//! globs), the template `result_digest` binding claim, and per-file
//! staged checks.

use crate::span::Span;
use crate::workspace::WorkspaceDestination;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a profile file's bytes originate (v6 grammar).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceSource {
    RepoFile {
        path: String,
    },
    /// A file inside another output's artifact — the output must exist
    /// in the catalog (sema E126).
    ArtifactFile {
        output: String,
        selector: String,
    },
    /// A subtree of another output's artifact, selected by include
    /// globs (min 1, sema E130) minus exclude globs. The output must
    /// be an artifact kind (recipe|package; sema E126/E128).
    Tree {
        output: String,
        include: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        exclude: Vec<String>,
    },
}

/// What a profile file contains — an axis orthogonal to both origin and
/// destination policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceContent {
    /// Source bytes, unmodified. Requires `source` (sema E130).
    Identity,
    /// Inline literal content — the only variant that may omit `source`.
    Literal { text: String },
    /// Rendered template. Requires `source` (sema E130). `result_digest`
    /// is an optional binding claim: 64 lowercase hex when present
    /// (sema E130); realization renders and verifies equality, absent
    /// means unbound (v6-design §2.3).
    Template {
        template: String,
        variables: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result_digest: Option<String>,
    },
}

/// A staged check over one profile file (v6-design §2.3): `check` names
/// a check output (sema E126, validation-role edge from the profile).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFileCheck {
    pub check: String,
    pub subject: FileCheckSubject,
    pub stage: FileCheckStage,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileCheckSubject {
    Source,
    Rendered,
    Deployed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileCheckStage {
    PreFlip,
    PostLink,
    PostActivate,
}

/// One owned v6 profile file: origin, content, destination policy and
/// staged checks composed independently, with mandatory provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFile {
    pub span: Span,
    /// Optional only for literal content (sema E130).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<WorkspaceSource>,
    pub content: WorkspaceContent,
    pub destination: WorkspaceDestination,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<WorkspaceFileCheck>,
}
