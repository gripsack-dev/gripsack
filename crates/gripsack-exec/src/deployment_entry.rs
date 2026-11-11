//! The runtime deployment entry: an immutable payload mapped to a
//! destination by profile preparation. Moved from `gripsack-store`
//! (plan/0056 F2 step 10b): it carries the IR's source span, so it lives
//! with its only user rather than in the host's store codecs.

use gripsack_host::store::Ownership;
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
