//! The portable v6 workspace lock (v6-design §2.4) — representation
//! only; the native worker owns all I/O. Fail-closed reader semantics
//! mirror the legacy lockfile: a wrong `lock_version`, malformed JSON
//! or a non-hex pin is rejected, never defaulted. No VM/socket/lease/
//! instance-path fields exist by construction. `deny_unknown_fields`
//! everywhere: this type family is persisted state, versioned by
//! `lock_version`.

pub mod conda;
mod decode;
mod platform;
use super::identity::DefinitionDigest;
use super::source::LockedSource;
pub use conda::{
    BytecodePolicy, ChannelPriority, LockedCondaEnvironment, LockedCondaPackage, LockedNoArch,
    LockedRunExports, LockedVirtualPackage, MaterializerPolicy, ReceiptPolicy,
};
pub use platform::{parse_platform_key, platform_key};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The only `lock_version` this reader admits. Version 2 is the Conda
/// cutover: location-free locked sources, frozen Conda closure records,
/// and no unproduced recipe/realized-tree authority fields.
pub const WORKSPACE_LOCK_VERSION: u32 = 2;


/// A versioned, platform-partitioned resolution record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceLock {
    pub lock_version: u32,
    pub definitions: DefinitionPins,
    /// Platform key (`platform_key` spelling) → that platform's pins.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "decode::unique_map"
    )]
    pub resolutions: BTreeMap<String, PlatformResolution>,
}

/// Captured compiler and explicitly pinned imported recipes. Local declaration
/// locations are provenance, not import pins or producer identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionPins {
    pub frontend: DefinitionDigest,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "decode::unique_map"
    )]
    pub imports: BTreeMap<String, DefinitionDigest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformResolution {
    pub pins: Vec<LockedPin>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitive: Vec<TransitivePin>,
}
/// One acquired output pin: the location-free declared source, the
/// resolved identity, and (for Conda sources) the frozen closure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedPin {
    /// Name of the workspace output this pin resolves.
    pub output: String,
    #[serde(deserialize_with = "decode::source")]
    pub source: LockedSource,
    pub resolved: ResolvedPinFields,
    /// Frozen Conda closure; present exactly when `source` is a
    /// Conda environment or Pixi lock import.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conda: Option<LockedCondaEnvironment>,
}

/// The acquired identity of a fetch — every field optional, every
/// present hash 64 lowercase hex.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedPinFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo256: Option<String>,
}

/// A resolved transitive dependency pin (not itself a workspace output).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitivePin {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Fail-closed lock-reader errors. Missing versus corrupt is the
/// caller's distinction (the I/O layer, native worker) — this reader
/// only ever sees bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkspaceLockError {
    /// Malformed JSON, an unknown field, or a structurally invalid
    /// value (including a locked source rejected by its decoder).
    #[error("corrupt workspace lock: {0}")]
    Corrupt(String),
    /// `lock_version` other than `WORKSPACE_LOCK_VERSION`.
    #[error("unsupported workspace lock version {0}")]
    UnsupportedVersion(u32),
    /// A semantically invalid pin: empty name or non-hex hash field.
    #[error("invalid pin in workspace lock: {0}")]
    InvalidPin(String),
}

/// 64 lowercase ASCII hex characters.
fn is_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn require_hex64(field: &str, value: &str) -> Result<(), WorkspaceLockError> {
    if is_hex64(value) {
        Ok(())
    } else {
        Err(WorkspaceLockError::InvalidPin(format!(
            "{field} must be 64 lowercase hex characters"
        )))
    }
}

fn require_hex64_opt(field: &str, value: &Option<String>) -> Result<(), WorkspaceLockError> {
    match value {
        Some(value) => require_hex64(field, value),
        None => Ok(()),
    }
}

impl WorkspaceLock {
    /// Fail-closed reader: JSON error → `Corrupt`; wrong version →
    /// `UnsupportedVersion`; then full pin validation.
    pub fn from_json(text: &str) -> Result<WorkspaceLock, WorkspaceLockError> {
        let lock: WorkspaceLock =
            serde_json::from_str(text).map_err(|e| WorkspaceLockError::Corrupt(e.to_string()))?;
        lock.validate()?;
        Ok(lock)
    }

    /// Full semantic validation, run by `from_json` after the version
    /// gate (and available to producers before persisting).
    pub fn validate(&self) -> Result<(), WorkspaceLockError> {
        if self.lock_version != WORKSPACE_LOCK_VERSION {
            return Err(WorkspaceLockError::UnsupportedVersion(self.lock_version));
        }
        for name in self.definitions.imports.keys() {
            if name.is_empty() || name.chars().any(char::is_control) {
                return Err(WorkspaceLockError::InvalidPin(
                    "invalid imported-definition identity".into(),
                ));
            }
        }
        for (platform, resolution) in &self.resolutions {
            let declared = parse_platform_key(platform)?;
            let mut names = std::collections::BTreeSet::new();
            for pin in &resolution.pins {
                if pin.output.is_empty()
                    || pin.output.chars().any(char::is_control)
                    || !names.insert(&pin.output)
                {
                    return Err(WorkspaceLockError::InvalidPin(format!(
                        "empty, invalid or duplicate output name under platform {platform}"
                    )));
                }
                if pin.resolved.sha256.is_none() && pin.resolved.tree256.is_none() {
                    return Err(WorkspaceLockError::InvalidPin(format!(
                        "output {:?} has no acquired byte/tree identity",
                        pin.output
                    )));
                }
                require_hex64_opt("resolved.sha256", &pin.resolved.sha256)?;
                require_hex64_opt("resolved.tree256", &pin.resolved.tree256)?;
                require_hex64_opt("resolved.repo256", &pin.resolved.repo256)?;
                validate_conda_pin(platform, &declared, pin)?;
            }
            let mut transitive = std::collections::BTreeSet::new();
            for pin in &resolution.transitive {
                if pin.name.is_empty()
                    || pin.name.chars().any(char::is_control)
                    || !transitive.insert(&pin.name)
                {
                    return Err(WorkspaceLockError::InvalidPin(format!(
                        "empty, invalid or duplicate transitive name under platform {platform}"
                    )));
                }
                require_hex64("transitive sha256", &pin.sha256)?;
            }
        }
        Ok(())
    }
}

/// Conda pin invariants (A3): the frozen closure is present exactly for
/// Conda sources, its recorded identity is the canonical closure digest,
/// and the records are complete, consistently ordered and bound to the
/// resolution's platform partition.
fn validate_conda_pin(
    platform: &str,
    declared: &crate::workspace::WorkspacePlatform,
    pin: &LockedPin,
) -> Result<(), WorkspaceLockError> {
    use crate::workspace::{PlatformArch, PlatformOs};
    let conda_source = matches!(
        pin.source,
        LockedSource::CondaEnvironment(_) | LockedSource::PixiLock(_)
    );
    let Some(environment) = &pin.conda else {
        if conda_source {
            return Err(WorkspaceLockError::InvalidPin(format!(
                "conda-source output {:?} has no frozen closure",
                pin.output
            )));
        }
        return Ok(());
    };
    if !conda_source {
        return Err(WorkspaceLockError::InvalidPin(format!(
            "fetch-source output {:?} carries a frozen conda closure",
            pin.output
        )));
    }
    if pin.resolved.tree256.is_some()
        || pin.resolved.url.is_some()
        || pin.resolved.version.is_some()
        || pin.resolved.api_url.is_some()
        || pin.resolved.repo256.is_some()
    {
        return Err(WorkspaceLockError::InvalidPin(format!(
            "conda output {:?} carries fetch-lane resolved fields",
            pin.output
        )));
    }
    let digest = super::identity::conda_closure_digest(environment).to_string();
    if pin.resolved.sha256.as_deref() != Some(digest.as_str()) {
        return Err(WorkspaceLockError::InvalidPin(format!(
            "conda output {:?} identity is not its canonical closure digest",
            pin.output
        )));
    }
    let (os, arch) = match environment.platform.as_str() {
        "linux-64" => (PlatformOs::Linux, PlatformArch::X86_64),
        "linux-aarch64" => (PlatformOs::Linux, PlatformArch::Aarch64),
        "osx-64" => (PlatformOs::Macos, PlatformArch::X86_64),
        "osx-arm64" => (PlatformOs::Macos, PlatformArch::Aarch64),
        other => {
            return Err(WorkspaceLockError::InvalidPin(format!(
                "conda output {:?} has unsupported subdir {other:?}",
                pin.output
            )));
        }
    };
    if declared.os != os || declared.arch != arch {
        return Err(WorkspaceLockError::InvalidPin(format!(
            "conda output {:?} subdir does not match platform {platform}",
            pin.output
        )));
    }
    if environment.channels.is_empty() {
        return Err(WorkspaceLockError::InvalidPin(format!(
            "conda output {:?} records no channels",
            pin.output
        )));
    }
    if environment.packages.is_empty() {
        return Err(WorkspaceLockError::InvalidPin(format!(
            "conda output {:?} records an empty closure",
            pin.output
        )));
    }
    let mut previous: Option<&str> = None;
    for package in &environment.packages {
        if package.name.is_empty()
            || package.name.chars().any(char::is_control)
            || previous.is_some_and(|name| name >= package.name.as_str())
        {
            return Err(WorkspaceLockError::InvalidPin(format!(
                "conda output {:?} closure is not in canonical name order",
                pin.output
            )));
        }
        previous = Some(&package.name);
        require_hex64("conda package sha256", &package.sha256)?;
        if package.subdir != environment.platform && package.subdir != "noarch" {
            return Err(WorkspaceLockError::InvalidPin(format!(
                "conda package {:?} subdir does not match its environment",
                package.name
            )));
        }
    }
    let mut virtuals = std::collections::BTreeSet::new();
    for package in &environment.virtual_packages {
        if package.name.is_empty() || !virtuals.insert(&package.name) {
            return Err(WorkspaceLockError::InvalidPin(format!(
                "conda output {:?} has empty or duplicate virtual packages",
                pin.output
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{OsVersion, PlatformAbi, PlatformArch, PlatformOs, WorkspacePlatform};

    fn hex(c: char) -> String {
        c.to_string().repeat(64)
    }

    fn lock_json(version: u32) -> serde_json::Value {
        serde_json::json!({
            "lock_version": version,
            "definitions": {"frontend": hex('f')},
            "resolutions": {
                "linux-x86_64": {
                    "pins": [{
                        "output": "toolchain",
                        "source": {"kind": "fetch", "fetch": {"kind": "tarball", "url": "https://example/t.tgz"}},
                        "resolved": {"url": "https://example/t.tgz", "version": "1.0", "sha256": hex('a')}
                    }],
                    "transitive": [{"name": "glibc", "version": "2.36", "sha256": hex('c')}]
                }
            }
        })
    }

    fn closure() -> LockedCondaEnvironment {
        LockedCondaEnvironment {
            platform: "linux-64".into(),
            channels: vec!["https://conda.anaconda.org/conda-forge".into()],
            channel_priority: ChannelPriority::Strict,
            virtual_packages: vec![LockedVirtualPackage {
                name: "__glibc".into(),
                version: "2.31.0".into(),
                build: "0".into(),
            }],
            packages: vec![
                LockedCondaPackage {
                    name: "libgcc-ng".into(),
                    version: "14.2.0".into(),
                    build: "h69a702a_2".into(),
                    build_number: 2,
                    subdir: "linux-64".into(),
                    channel: "https://conda.anaconda.org/conda-forge".into(),
                    url: "https://conda.anaconda.org/conda-forge/linux-64/libgcc-ng-14.2.0-h69a702a_2.conda".into(),
                    sha256: hex('1'),
                    size: Some(100),
                    timestamp: None,
                    indexed_timestamp: None,
                    attestations_sha256: None,
                    md5: None,
                    legacy_bz2_md5: None,
                    legacy_bz2_size: None,
                    arch: Some("x86_64".into()),
                    platform: Some("linux".into()),
                    noarch: LockedNoArch::None,
                    license: None,
                    license_family: None,
                    depends: vec![],
                    constrains: vec![],
                    extra_depends: BTreeMap::new(),
                    flags: Vec::new(),
                    python_site_packages_path: None,
                    run_exports: None,
                    purls: None,
                    track_features: vec![],
                    features: None,
                },
                LockedCondaPackage {
                    name: "python".into(),
                    version: "3.12.7".into(),
                    build: "hc5c86c4_0".into(),
                    build_number: 0,
                    subdir: "linux-64".into(),
                    channel: "https://conda.anaconda.org/conda-forge".into(),
                    url: "https://conda.anaconda.org/conda-forge/linux-64/python-3.12.7-hc5c86c4_0.conda".into(),
                    sha256: hex('2'),
                    size: None,
                    timestamp: None,
                    indexed_timestamp: None,
                    attestations_sha256: None,
                    md5: None,
                    legacy_bz2_md5: None,
                    legacy_bz2_size: None,
                    arch: None,
                    platform: None,
                    noarch: LockedNoArch::None,
                    license: None,
                    license_family: None,
                    depends: vec!["libgcc-ng >=14.2.0".into(), "__glibc >=2.17,<3.0.a0".into()],
                    constrains: vec![],
                    extra_depends: BTreeMap::new(),
                    flags: Vec::new(),
                    python_site_packages_path: None,
                    run_exports: None,
                    purls: None,
                    track_features: vec![],
                    features: None,
                },
            ],
            materializer: MaterializerPolicy {
                bytecode: BytecodePolicy::Suppress,
                receipt: ReceiptPolicy::NormalizedCondaMeta,
            },
        }
    }

    fn conda_lock_json() -> serde_json::Value {
        let closure = closure();
        let digest = super::super::identity::conda_closure_digest(&closure).to_string();
        let mut json = lock_json(2);
        json["resolutions"]["linux-x86_64"]["pins"][0]["source"] = serde_json::json!({
            "kind": "conda_environment",
            "channels": ["conda-forge"],
            "packages": {"python": "3.12.*"}
        });
        json["resolutions"]["linux-x86_64"]["pins"][0]["resolved"] =
            serde_json::json!({"sha256": digest});
        json["resolutions"]["linux-x86_64"]["pins"][0]["conda"] =
            serde_json::to_value(&closure).unwrap();
        json
    }

    #[test]
    fn from_json_rejects_bad_version() {
        assert_eq!(
            WorkspaceLock::from_json(&lock_json(1).to_string()),
            Err(WorkspaceLockError::UnsupportedVersion(1))
        );
    }

    #[test]
    fn conda_closure_round_trips_and_binds_its_canonical_digest() {
        let lock = WorkspaceLock::from_json(&conda_lock_json().to_string()).unwrap();
        let pin = &lock.resolutions["linux-x86_64"].pins[0];
        assert!(matches!(pin.source, LockedSource::CondaEnvironment(_)));
        assert_eq!(pin.conda.as_ref().unwrap().packages.len(), 2);
        let mut drifted = conda_lock_json();
        drifted["resolutions"]["linux-x86_64"]["pins"][0]["resolved"]["sha256"] =
            serde_json::json!(hex('9'));
        assert!(matches!(
            WorkspaceLock::from_json(&drifted.to_string()),
            Err(WorkspaceLockError::InvalidPin(_))
        ));
    }

    #[test]
    fn conda_closure_invariants_are_fail_closed() {
        // A fetch pin may not carry a frozen closure.
        let mut json = conda_lock_json();
        json["resolutions"]["linux-x86_64"]["pins"][0]["source"] = serde_json::json!({
            "kind": "fetch", "fetch": {"kind": "tarball", "url": "https://example/t.tgz"}
        });
        assert!(matches!(
            WorkspaceLock::from_json(&json.to_string()),
            Err(WorkspaceLockError::InvalidPin(_))
        ));
        // A conda pin may not drop its closure.
        let mut json = conda_lock_json();
        json["resolutions"]["linux-x86_64"]["pins"][0]
            .as_object_mut()
            .unwrap()
            .remove("conda");
        assert!(matches!(
            WorkspaceLock::from_json(&json.to_string()),
            Err(WorkspaceLockError::InvalidPin(_))
        ));
        // The closure must stay in canonical name order.
        let mut json = conda_lock_json();
        json["resolutions"]["linux-x86_64"]["pins"][0]["conda"]["packages"]
            .as_array_mut()
            .unwrap()
            .reverse();
        assert!(matches!(
            WorkspaceLock::from_json(&json.to_string()),
            Err(WorkspaceLockError::InvalidPin(_))
        ));
        // A conda pin may not carry fetch-lane resolved fields.
        let mut json = conda_lock_json();
        json["resolutions"]["linux-x86_64"]["pins"][0]["resolved"]["tree256"] =
            serde_json::json!(hex('8'));
        assert!(matches!(
            WorkspaceLock::from_json(&json.to_string()),
            Err(WorkspaceLockError::InvalidPin(_))
        ));
        // The subdir must match the platform partition.
        let mut json = conda_lock_json();
        json["resolutions"]["linux-x86_64"]["pins"][0]["conda"]["platform"] =
            serde_json::json!("osx-arm64");
        assert!(matches!(
            WorkspaceLock::from_json(&json.to_string()),
            Err(WorkspaceLockError::InvalidPin(_))
        ));
    }

    #[test]
    fn from_json_rejects_malformed_json() {
        assert!(matches!(
            WorkspaceLock::from_json("{not json"),
            Err(WorkspaceLockError::Corrupt(_))
        ));
    }

    #[test]
    fn from_json_rejects_unknown_field() {
        let mut json = lock_json(2);
        json["surprise"] = serde_json::json!(true);
        assert!(matches!(
            WorkspaceLock::from_json(&json.to_string()),
            Err(WorkspaceLockError::Corrupt(_))
        ));
    }

    #[test]
    fn from_json_rejects_bad_hex() {
        for (path, bad) in [
            ("resolved-sha256", "resolved"),
            
            ("transitive-sha256", "transitive"),
        ] {
            let mut json = lock_json(2);
            let pin = &mut json["resolutions"]["linux-x86_64"];
            match path {
                "resolved-sha256" => {
                    pin["pins"][0]["resolved"]["sha256"] =
                        serde_json::json!("AB".to_string() + &"cd".repeat(31))
                }
                _ => pin["transitive"][0]["sha256"] = serde_json::json!("short"),
            }
            let _ = bad;
            assert!(
                matches!(
                    WorkspaceLock::from_json(&json.to_string()),
                    Err(WorkspaceLockError::InvalidPin(_) | WorkspaceLockError::Corrupt(_))
                ),
                "{path} must be rejected"
            );
        }
    }

    #[test]
    fn from_json_rejects_empty_names() {
        let mut json = lock_json(2);
        json["resolutions"]["linux-x86_64"]["pins"][0]["output"] = serde_json::json!("");
        assert!(matches!(
            WorkspaceLock::from_json(&json.to_string()),
            Err(WorkspaceLockError::InvalidPin(_))
        ));
        let mut json = lock_json(2);
        json["resolutions"]["linux-x86_64"]["transitive"][0]["name"] = serde_json::json!("");
        assert!(matches!(
            WorkspaceLock::from_json(&json.to_string()),
            Err(WorkspaceLockError::InvalidPin(_))
        ));
    }

    #[test]
    fn equivalent_platform_floors_share_one_lock_partition() {
        let mut platform = WorkspacePlatform {
            os: PlatformOs::Macos,
            arch: PlatformArch::Aarch64,
            abi: Some(PlatformAbi::Darwin),
            minimum_os: Some(OsVersion {
                major: 14,
                minor: 0,
                patch: None,
            }),
        };
        let omitted = platform_key(&platform);
        platform.minimum_os.as_mut().unwrap().patch = Some(0);
        assert_eq!(omitted, platform_key(&platform));
        assert_eq!(parse_platform_key(&omitted).unwrap(), platform);
        platform.minimum_os.as_mut().unwrap().patch = Some(1);
        assert_ne!(omitted, platform_key(&platform));
        for key in [
            "linux-x86_64-darwin",
            "macos-aarch64-gnu",
            "linux-x86_64@01.2.3",
            "macos-aarch64@14.0",
            "other-x86_64",
        ] {
            assert!(parse_platform_key(key).is_err(), "{key}");
        }
    }

    #[test]
    fn incomplete_and_conflicting_pins_cannot_be_used_as_a_frozen_lock() {
        let mut missing = lock_json(2);
        missing["resolutions"]["linux-x86_64"]["pins"][0]["resolved"] =
            serde_json::json!({"version": "floating"});
        assert!(matches!(
            WorkspaceLock::from_json(&missing.to_string()),
            Err(WorkspaceLockError::InvalidPin(_))
        ));
        let mut duplicate = lock_json(2);
        let pins = duplicate["resolutions"]["linux-x86_64"]["pins"]
            .as_array_mut()
            .unwrap();
        let mut changed = pins[0].clone();
        changed["resolved"]["sha256"] = hex('d').into();
        pins.push(changed);
        assert!(matches!(
            WorkspaceLock::from_json(&duplicate.to_string()),
            Err(WorkspaceLockError::InvalidPin(_))
        ));
        let mut transitive = lock_json(2);
        let pins = transitive["resolutions"]["linux-x86_64"]["transitive"]
            .as_array_mut()
            .unwrap();
        pins.push(pins[0].clone());
        assert!(matches!(
            WorkspaceLock::from_json(&transitive.to_string()),
            Err(WorkspaceLockError::InvalidPin(_))
        ));
    }

    #[test]
    fn duplicate_platforms_and_unknown_fetch_fields_do_not_disappear_during_decode() {
        let fixture = lock_json(2);
        let resolution = &fixture["resolutions"]["linux-x86_64"];
        let duplicate = format!(
            r#"{{"lock_version":2,"definitions":{},"resolutions":{{"linux-x86_64":{resolution},"linux-x86_64":{resolution}}}}}"#,
            fixture["definitions"]
        );
        assert!(matches!(
            WorkspaceLock::from_json(&duplicate),
            Err(WorkspaceLockError::Corrupt(_))
        ));
        let mut unknown = fixture;
        unknown["resolutions"]["linux-x86_64"]["pins"][0]["fetch"]["allow_network"] = true.into();
        assert!(matches!(
            WorkspaceLock::from_json(&unknown.to_string()),
            Err(WorkspaceLockError::Corrupt(_))
        ));
    }
}
