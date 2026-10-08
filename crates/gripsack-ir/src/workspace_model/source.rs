//! Shared catalog acquisition model: the existing archive/git/file
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

/// Optional fields may be absent, but explicit null is not a declaration.
pub(super) fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// How a v6 recipe or provider package obtains its payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AcquisitionSource {
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
    /// Linux solve baseline, inherited by frozen runtime admission. Omitted
    /// capabilities retain measured solve facts, not implicit runtime floors.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub system_requirements: Option<CondaSystemRequirements>,
    pub span: Span,
}

/// Explicit native Conda solve floors. Nested values inherit the source span.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CondaSystemRequirements {
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub libc: Option<CondaLibcRequirement>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub linux: Option<String>,
}

/// Linux libc baseline. Only the `glibc` family is supported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CondaLibcRequirement {
    pub family: String,
    pub version: String,
}

impl CondaSystemRequirements {
    /// Admit decimal, dot-separated Linux versions, not MatchSpecs, wildcards,
    /// vendor suffixes or other libc families.
    pub fn validate(&self) -> Result<(), &'static str> {
        let version = |value: &str| {
            value
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        };
        if let Some(libc) = &self.libc {
            if libc.family != "glibc" {
                return Err("conda system requirements support only the glibc libc family");
            }
            if !version(&libc.version) {
                return Err("conda libc version must be a decimal dot-separated version");
            }
        }
        if self.linux.as_deref().is_some_and(|value| !version(value)) {
            return Err("conda linux version must be a decimal dot-separated version");
        }
        Ok(())
    }

    /// Preserve declared floors in the existing lock/receipt policy shape.
    pub fn locked(&self) -> Result<super::lock::LockedCondaSystemRequirements, &'static str> {
        self.validate()?;
        let mut virtual_packages = Vec::with_capacity(
            usize::from(self.libc.is_some()) + usize::from(self.linux.is_some()),
        );
        for (name, version) in [
            ("__glibc", self.libc.as_ref().map(|libc| &libc.version)),
            ("__linux", self.linux.as_ref()),
        ] {
            if let Some(version) = version {
                virtual_packages.push(super::lock::LockedVirtualPackageRequirement {
                    name: name.into(),
                    minimum_version: version.clone(),
                    build: None,
                });
            }
        }
        Ok(super::lock::LockedCondaSystemRequirements {
            virtual_packages,
            archspec: None,
        })
    }
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

impl AcquisitionSource {
    /// Whether this declaration requires the strict v7 wire grammar.
    pub fn requires_v7(&self) -> bool {
        matches!(self, Self::CondaEnvironment(source) if source.system_requirements.is_some())
    }

    /// The declaration span of this source, for diagnostics.
    pub fn span(&self) -> &Span {
        match self {
            AcquisitionSource::Fetch(fetch) => &fetch.span,
            AcquisitionSource::CondaEnvironment(source) => &source.span,
            AcquisitionSource::PixiLock(source) => &source.span,
        }
    }

    /// Location-free acquisition projection persisted by the portable lock.
    /// Conda system requirements live in the pin's existing closure policy;
    /// frozen callers must compare that policy as well as this projection.
    pub fn locked(&self) -> LockedSource {
        match self {
            AcquisitionSource::Fetch(fetch) => LockedSource::Fetch {
                fetch: fetch.fetch.clone(),
            },
            AcquisitionSource::CondaEnvironment(source) => {
                LockedSource::CondaEnvironment(LockedCondaSource {
                    channels: source.channels.clone(),
                    packages: source.packages.clone(),
                    platforms: source.platforms.clone(),
                })
            }
            AcquisitionSource::PixiLock(source) => LockedSource::PixiLock(LockedPixiSource {
                manifest: source.manifest.clone(),
                lock: source.lock.clone(),
                environment: source.environment.clone(),
                manifest_sha256: None,
                lock_sha256: None,
            }),
        }
    }
}

/// The location-free source record persisted in the portable lock. Conda
/// baseline semantics are carried separately by the pin's closure policy.
/// `deny_unknown_fields` applies per variant struct; the `fetch` variant is
/// closed by the lock decoder's per-kind allowlist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LockedSource {
    /// The fetch spec nests under its own field: it is itself an
    /// internally tagged enum, so it cannot share the outer `kind`.
    Fetch {
        fetch: FetchSpec,
    },
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_system_requirements_preserve_strict_declared_floors() {
        let value = json!({"libc":{"family":"glibc","version":"2.28"},"linux":"4.18"});
        let requirements: CondaSystemRequirements = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&requirements).unwrap(), value);
        let locked = requirements.locked().unwrap();
        assert_eq!(locked.virtual_packages.len(), 2);
        assert_eq!(locked.virtual_packages[0].name, "__glibc");
        assert_eq!(locked.virtual_packages[0].minimum_version, "2.28");
        assert_eq!(locked.virtual_packages[1].name, "__linux");
        assert_eq!(locked.virtual_packages[1].minimum_version, "4.18");
        assert_eq!(requirements, serde_json::from_value(value).unwrap());
        assert_eq!(
            CondaSystemRequirements::default().locked().unwrap(),
            Default::default()
        );
    }

    #[test]
    fn unsupported_baseline_fields_families_and_versions_are_rejected() {
        for value in [
            json!({"cuda":"12"}),
            json!({"libc":{"family":"glibc","version":"2.28","build":"0"}}),
            json!({"libc":{"version":"2.28"}}),
            json!({"libc":null}),
            json!({"linux":null}),
        ] {
            assert!(serde_json::from_value::<CondaSystemRequirements>(value).is_err());
        }
        for version in [
            "", "2.", ".28", "2..28", ">=2.28", "2.*", "2.28\n", " 2.28", "2.28-gnu",
        ] {
            for value in [
                json!({"libc":{"family":"glibc","version":version}}),
                json!({"linux":version}),
            ] {
                let requirements: CondaSystemRequirements = serde_json::from_value(value).unwrap();
                assert!(requirements.locked().is_err(), "{version:?}");
            }
        }
        let requirements: CondaSystemRequirements =
            serde_json::from_value(json!({"libc":{"family":"musl","version":"1.2"}})).unwrap();
        assert!(requirements.locked().is_err());
    }

    #[test]
    fn absent_baseline_retains_source_shape_but_null_is_not_absence() {
        let value = json!({
            "channels":["conda-forge"], "packages":{"python":"*"},
            "span":{"file":"workspace.ts","line":1}
        });
        let source: CondaEnvironmentSource = serde_json::from_value(value.clone()).unwrap();
        assert!(source.system_requirements.is_none());
        assert_eq!(serde_json::to_value(source).unwrap(), value);
        let mut invalid = value;
        invalid["system_requirements"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<CondaEnvironmentSource>(invalid).is_err());
    }
}
