//! Persistent OCI service declarations. These describe a managed container,
//! never a recipe, host process fallback or permission to delete named volumes.

use crate::Span;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Bound untrusted container configuration before contacting the engine.
pub const MAX_SERVICE_ENTRIES: usize = 128;
pub const MAX_SERVICE_ARGUMENTS: usize = 1024;
pub const MAX_SERVICE_VALUE_BYTES: usize = 65_536;
pub const MAX_SERVICE_HEALTH_SECONDS: u32 = 3600;
pub const MAX_SERVICE_HEALTH_RETRIES: u32 = 100;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceOutput {
    pub name: String,
    pub span: Span,
    /// Immutable registry reference ending in @sha256:<64 lowercase hex>.
    pub image: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<ServicePort>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub volumes: Vec<ServiceVolume>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, ServiceEnvValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    pub health: ServiceHealth,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServicePort {
    pub host: u16,
    pub container: u16,
    /// Explicit canonical loopback address; never all interfaces by default.
    pub address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceVolume {
    /// Engine-managed volume identity, not a host filesystem path.
    pub name: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServiceEnvValue {
    Literal {
        value: String,
    },
    /// Resolve explicitly at launch, never record the inherited secret value.
    Inherited {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServiceHealth {
    Command {
        argv: Vec<String>,
        interval_seconds: u32,
        timeout_seconds: u32,
        retries: u32,
    },
}

/// Admit an immutable OCI registry reference without resolving tags or URLs.
pub fn valid_service_image(reference: &str) -> bool {
    let Some((name, digest)) = reference.rsplit_once("@sha256:") else {
        return false;
    };
    !name.is_empty()
        && name.len() <= 255
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-/:".contains(&byte)
        })
        && name
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && !name.ends_with('/')
        && !name.ends_with(':')
        && !name.contains("//")
        && digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
