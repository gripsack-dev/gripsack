use gripsack_ir::workspace_model::identity::DefinitionDigest;
use serde::{Deserialize, Serialize};

pub const SNAPSHOT_VERSION: u32 = 1;
pub const CANONICALIZATION_VERSION: u32 = 1;
pub const POLICY_VERSION: u32 = 1;
/// Admission budgets, not performance claims. Complete affected sets fit these bounds.
pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_NODES: usize = 10_000;
pub const MAX_EDGES: usize = 100_000;
pub const MAX_FACTS: usize = 100_000;
pub const MAX_DEPTH: usize = 64;
pub const MAX_TEXT_BYTES: usize = 1024;
pub const MAX_REASONS: usize = 10_000;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("semantic snapshot exceeds {0} bound")]
    Bound(&'static str),
    #[error("unsupported semantic snapshot {0}")]
    Version(u32),
    #[error("invalid semantic snapshot: {0}")]
    Invalid(&'static str),
    #[error("malformed semantic snapshot JSON")]
    Json(#[from] serde_json::Error),
    #[error("semantic snapshot I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SubjectRef(pub String);
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldPath(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotOrigin {
    Current,
    Generation,
    Proposed,
    Recorded,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    Workspace,
    Output,
    Input,
    Deployment,
    Job,
    Distribution,
    Package,
    Recipe,
    Environment,
    Task,
    Schedule,
    Check,
    Profile,
    Hook,
    Service,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Declaration,
    Input,
    SelectedArtifact,
    Materialization,
    Environment,
    Deployment,
    Validation,
    Eligibility,
    Provenance,
    Invocation,
    Registration,
    JobRevision,
    Observation,
    Ordering,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Production,
    BuildInput,
    Runtime,
    Ordering,
    TaskPrerequisite,
    Validation,
    Retention,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceOrigin {
    Declaration,
    Lock,
    Receipt,
    Observation,
    Historical,
    Uncaptured,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    MissingLock,
    MissingInput,
    MissingProbe,
    UnavailableReceipt,
    HistoricalDeclaration,
    MutableExternalState,
    OpaqueEffect,
    Redacted,
    UnsupportedContract,
    Unobserved,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum KnowledgeState {
    /// Review equality evidence, never recipe identity or execution authority.
    Known {
        digest: DefinitionDigest,
    },
    Unknown {
        reason: UnknownReason,
    },
}
impl KnowledgeState {
    pub fn is_known(&self) -> bool {
        matches!(self, Self::Known { .. })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub path: FieldPath,
    pub kind: ChangeKind,
    pub knowledge: KnowledgeState,
    pub origin: EvidenceOrigin,
    /// Unix seconds of the original observation; None means age is not known.
    pub observed_at: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// Opaque location binding: no filesystem path or credential URL disclosure.
    pub source: DefinitionDigest,
    /// Safe relative source spelling only. Absolute and credential locations are omitted.
    pub source_label: Option<String>,
    pub line: u32,
    pub col: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotNode {
    pub id: SubjectRef,
    pub kind: SubjectKind,
    pub facts: Vec<Fact>,
    pub provenance: Vec<Provenance>,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotEdge {
    pub dependency: SubjectRef,
    pub consumer: SubjectRef,
    pub kind: EdgeKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Completeness {
    pub complete: bool,
    pub redacted: bool,
    pub truncated: bool,
    pub unknown_reasons: Vec<UnknownReason>,
}
/// Plain wire/builder DTO. Only `SemanticSnapshot::admit` grants comparison access.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotDocument {
    pub version: u32,
    pub canonicalization_version: u32,
    pub policy_version: u32,
    pub origin: SnapshotOrigin,
    pub generation: Option<String>,
    pub nodes: Vec<SnapshotNode>,
    pub edges: Vec<SnapshotEdge>,
    pub completeness: Completeness,
}
#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct SemanticSnapshot(pub(super) SnapshotDocument);
impl SemanticSnapshot {
    pub fn admit(document: SnapshotDocument) -> Result<Self, SnapshotError> {
        super::admission::admit(document)
    }
    pub fn document(&self) -> &SnapshotDocument {
        &self.0
    }
    pub fn into_document(self) -> SnapshotDocument {
        self.0
    }
}
impl SnapshotDocument {
    pub fn new(origin: SnapshotOrigin) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            canonicalization_version: CANONICALIZATION_VERSION,
            policy_version: POLICY_VERSION,
            origin,
            generation: None,
            nodes: Vec::new(),
            edges: Vec::new(),
            completeness: Completeness {
                complete: true,
                redacted: false,
                truncated: false,
                unknown_reasons: Vec::new(),
            },
        }
    }
}
