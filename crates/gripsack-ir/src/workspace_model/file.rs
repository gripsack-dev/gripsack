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
    /// Captured repository subtree with normalized segment-prefix selection.
    /// Empty includes select every entry; exclusions take precedence.
    RepoTree {
        path: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        include: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        exclude: Vec<String>,
    },
    /// A file inside another output's artifact (sema E126). Release/platform
    /// placeholders bind to that output's locked source (core 0.55+).
    ArtifactFile {
        output: String,
        selector: String,
    },
    /// Artifact subtree selected by normalized segment prefixes, not globs.
    /// Includes (min 1, sema E130) and excludes bind release/platform placeholders
    /// to the referenced output's locked source (core 0.55+).
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
    /// Render the captured source bytes, retaining their executable class.
    TemplateFile {
        variables: BTreeMap<String, String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "super::source::present"
        )]
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
