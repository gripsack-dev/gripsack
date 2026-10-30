//! Versioned persisted lock wire admission. Borrow source/closure JSON until
//! the version is known, retaining duplicate-key checks and strict v2 fields
//! without cloning an entire JSON tree or rewriting retained bytes.
use super::*;
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireLock<'a> {
    lock_version: u32,
    definitions: DefinitionPins,
    #[serde(default, borrow, deserialize_with = "super::decode::unique_map")]
    resolutions: BTreeMap<String, WireResolution<'a>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResolution<'a> {
    #[serde(borrow)]
    pins: Vec<WirePin<'a>>,
    #[serde(default)]
    transitive: Vec<TransitivePin>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePin<'a> {
    output: String,
    #[serde(borrow)]
    source: &'a RawValue,
    resolved: ResolvedPinFields,
    #[serde(default, borrow)]
    conda: Option<&'a RawValue>,
}

/// Frozen v2 field inventory. A wheel field is unknown even when it is empty;
/// neither a new writer's defaults nor typed projection widen this reader.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedCondaEnvironment {
    platform: String,
    channels: Vec<String>,
    channel_priority: ChannelPriority,
    system_requirements: LockedCondaSystemRequirements,
    #[serde(default)]
    virtual_packages: Vec<LockedVirtualPackage>,
    packages: Vec<LockedCondaPackage>,
    materializer: MaterializerPolicy,
}

fn corrupt(error: serde_json::Error) -> WorkspaceLockError {
    WorkspaceLockError::Corrupt(error.to_string())
}

fn conda(raw: &RawValue, version: u32) -> Result<LockedCondaEnvironment, WorkspaceLockError> {
    if version != 2 {
        return serde_json::from_str(raw.get()).map_err(corrupt);
    }
    let old: RetainedCondaEnvironment = serde_json::from_str(raw.get()).map_err(corrupt)?;
    Ok(LockedCondaEnvironment {
        platform: old.platform,
        channels: old.channels,
        channel_priority: old.channel_priority,
        system_requirements: old.system_requirements,
        virtual_packages: old.virtual_packages,
        packages: old.packages,
        wheels: Vec::new(),
        materializer: old.materializer,
    })
}

pub(super) fn decode(text: &str) -> Result<WorkspaceLock, WorkspaceLockError> {
    let wire: WireLock<'_> = serde_json::from_str(text).map_err(corrupt)?;
    if !matches!(wire.lock_version, 2 | WORKSPACE_LOCK_VERSION) {
        return Err(WorkspaceLockError::UnsupportedVersion(wire.lock_version));
    }
    let version = wire.lock_version;
    let mut resolutions = BTreeMap::new();
    for (platform, resolution) in wire.resolutions {
        let mut pins = Vec::with_capacity(resolution.pins.len());
        for pin in resolution.pins {
            let mut decoder = serde_json::Deserializer::from_str(pin.source.get());
            let source = super::decode::source_version(&mut decoder, version).map_err(corrupt)?;
            let conda = pin.conda.map(|raw| conda(raw, version)).transpose()?;
            pins.push(LockedPin {
                output: pin.output,
                source,
                resolved: pin.resolved,
                conda,
            });
        }
        resolutions.insert(
            platform,
            PlatformResolution {
                pins,
                transitive: resolution.transitive,
            },
        );
    }
    Ok(WorkspaceLock {
        lock_version: version,
        definitions: wire.definitions,
        resolutions,
    })
}
