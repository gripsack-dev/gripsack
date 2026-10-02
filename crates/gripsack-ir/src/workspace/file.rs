//! Profile file declarations (0052 §2.2 FileDecl): origin, content and
//! destination policy are three orthogonal axes composed per file, plus
//! the calendar grammar for schedule triggers. No template-ownership
//! conflation; provenance is mandatory on every file.

use crate::span::Span;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a profile file's bytes originate.
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
}

/// What a profile file contains — an axis orthogonal to both origin and
/// destination policy (0052 §2.2 FileDecl).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceContent {
    /// Source bytes, unmodified. Requires `source` (sema E130).
    Identity,
    /// Inline literal content — the only variant that may omit `source`.
    Literal { text: String },
    /// Rendered template. Requires `source` (sema E130).
    Template {
        template: String,
        variables: BTreeMap<String, String>,
    },
}

/// How a profile file reaches its destination — ownership policy
/// orthogonal to content (0052 §2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceDestination {
    Symlink { path: String },
    TrackedCopy { path: String },
    ManagedBlock { path: String, marker: String },
}

/// One owned profile file: origin, content and destination policy
/// composed independently, with mandatory provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFile {
    pub span: Span,
    /// Optional only for literal content (sema E130).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<WorkspaceSource>,
    pub content: WorkspaceContent,
    pub destination: WorkspaceDestination,
}

/// The versioned workspace calendar admits daily or weekly local time.
/// Named timezones, intervals, cron and system scope are rejected by
/// grammar, not admitted as inert promises (0052 §2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceCalendar {
    Daily { time: String },
    Weekly { weekday: Weekday, time: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}
