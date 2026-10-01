//! Current v6 acquisition grammar (A3): the existing archive/git/file
//! provider lane plus coherent Conda environments solved through Rattler
//! and explicit Pixi lock imports. This is a clean cutover, not a reader
//! extension: the frozen v5 `WorkspaceFetch` stays with the v5 reader,
//! and v6 sema rejects the legacy Brew/Pixi fetch spellings that the
//! Conda lanes supersede. Declaration types carry provenance spans; the
//! portable lock keeps a location-free projection (`LockedSource`) — no
//! spans, no host paths, versioned by `lock_version`.

use crate::model::FetchSpec;
use crate::span::Span;
use crate::workspace::{WorkspaceFetch, WorkspacePlatform};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How a v6 recipe or provider package obtains its payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceSourceV6 {
    /// Existing archive/git/github/file/plugin acquisition grammar
    /// (carries the shared fetch spec and its provenance span).
    Fetch(WorkspaceFetch),
    /// Coherent binary Conda environment solved through Rattler.
    CondaEnvironment(CondaEnvironmentSource),
    /// Explicit Pixi lock import (manifest satisfaction + platform
    /// selection against an already-resolved lock document).
    PixiLock(PixiLockSource),
}

/// A binary Conda environment request. Resolution happens only inside
/// the optional Rattler helper on explicit update; frozen consumers
/// materialize the locked closure without solving.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CondaEnvironmentSource {
    /// Channel names/URLs in priority order (e.g. "conda-forge").
    pub channels: Vec<String>,
    /// Package name → version MatchSpec (e.g. "3.12.*"); "*" allowed.
    pub packages: BTreeMap<String, String>,
    /// Declared target platforms; empty = resolved per requesting
    /// consumer platform.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<WorkspacePlatform>,
    pub span: Span,
}

/// An explicit Pixi lock import: the manifest and lock are workspace
/// input names (InputOrigin files), never host paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PixiLockSource {
    /// Workspace input name of the pixi manifest.
    pub manifest: String,
    /// Workspace input name of the pixi lock document.
    pub lock: String,
    /// Pixi environment name selected from the lock.
    pub environment: String,
    pub span: Span,
}

impl WorkspaceSourceV6 {
    /// The declaration span of this source, for diagnostics.
    pub fn span(&self) -> &Span {
        match self {
            WorkspaceSourceV6::Fetch(fetch) => &fetch.span,
            WorkspaceSourceV6::CondaEnvironment(source) => &source.span,
            WorkspaceSourceV6::PixiLock(source) => &source.span,
        }
    }

    /// Location-free projection persisted by the portable lock.
    pub fn locked(&self) -> LockedSource {
        match self {
            WorkspaceSourceV6::Fetch(fetch) => LockedSource::Fetch {
                fetch: fetch.fetch.clone(),
            },
            WorkspaceSourceV6::CondaEnvironment(source) => {
                LockedSource::CondaEnvironment(LockedCondaSource {
                    channels: source.channels.clone(),
                    packages: source.packages.clone(),
                    platforms: source.platforms.clone(),
                })
            }
            WorkspaceSourceV6::PixiLock(source) => LockedSource::PixiLock(LockedPixiSource {
                manifest: source.manifest.clone(),
                lock: source.lock.clone(),
                environment: source.environment.clone(),
                manifest_sha256: None,
                lock_sha256: None,
            }),
        }
    }
}

/// The location-free source record persisted in the portable lock:
/// identical semantics to the declared source without spans or host
/// locations. `deny_unknown_fields` applies per variant struct; the
/// `fetch` variant is closed by the lock decoder's per-kind allowlist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LockedSource {
    /// The fetch spec nests under its own field: it is itself an
    /// internally tagged enum, so it cannot share the outer `kind`.
    Fetch { fetch: FetchSpec },
    CondaEnvironment(LockedCondaSource),
    PixiLock(LockedPixiSource),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedCondaSource {
    pub channels: Vec<String>,
    pub packages: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<WorkspacePlatform>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedPixiSource {
    pub manifest: String,
    pub lock: String,
    pub environment: String,
    /// Captured content identities of the two inputs at update time
    /// (64 lowercase hex). Frozen preparation re-captures and compares:
    /// a changed manifest or lock document can never silently reuse a
    /// closure solved against different bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_sha256: Option<String>,
}
