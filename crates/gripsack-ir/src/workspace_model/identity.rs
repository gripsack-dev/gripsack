//! Content-bound semantic identities. Source locations and operational worker,
//! lease and invocation identities never enter producer identity. Resolved byte
//! bindings are supplied by the core; TypeScript pin claims are not authority.
mod encode;
use super::lock::{DefinitionPins, ResolvedPinFields, WorkspaceLock};
use super::{CheckOutput, PackageOutput, RecipeExecution, RecipeOutput, WorkspaceCatalog};
use crate::HostFacts;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("identity must be exactly 64 lowercase hexadecimal characters")]
pub struct InvalidDigest;
macro_rules! digest {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        pub struct $name([u8; 32]);
        impl $name {
            pub fn parse(text: &str) -> Result<Self, InvalidDigest> {
                if text.len() != 64 {
                    return Err(InvalidDigest);
                }
                let digit = |value| match value {
                    b'0'..=b'9' => Ok(value - b'0'),
                    b'a'..=b'f' => Ok(value - b'a' + 10),
                    _ => Err(InvalidDigest),
                };
                let mut bytes = [0; 32];
                for (output, pair) in bytes.iter_mut().zip(text.as_bytes().as_chunks::<2>().0) {
                    *output = digit(pair[0])? << 4 | digit(pair[1])?;
                }
                Ok(Self(bytes))
            }
            pub fn bytes(&self) -> &[u8; 32] {
                &self.0
            }
            pub fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }
        }
        impl AsRef<[u8; 32]> for $name {
            fn as_ref(&self) -> &[u8; 32] {
                &self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
                for byte in self.0 {
                    write!(output, "{byte:02x}")?;
                }
                Ok(())
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
                let text = String::deserialize(decoder)?;
                Self::parse(&text).map_err(serde::de::Error::custom)
            }
        }
    };
}
digest!(RecipeDigest);
digest!(PackageDigest);
digest!(LockDigest);
digest!(PlanDigest);
digest!(CheckDigest);
digest!(ArtifactDigest);
digest!(ExecutableDigest);
digest!(DefinitionDigest);
digest!(ProductionDigest);
digest!(PackageProductionDigest);

/// A not-yet-built tool is bound to its admitted production, never an invented
/// executable checksum. Concrete byte claims remain validation obligations.
#[derive(Debug, Clone, Copy)]
pub enum ToolExecutable {
    Captured(ExecutableDigest),
    Produced(ProductionDigest),
}

#[derive(Debug, Clone)]
pub struct ToolPin {
    pub executable: ToolExecutable,
    pub package: PackageProductionDigest,
    pub selector: String,
}
/// Bindings have semantic values, not declaration labels. Names are lookup
/// keys only; the canonical encoder writes their resolved identities.
#[derive(Debug, Clone, Default)]
pub struct CommandPins {
    pub tools: BTreeMap<String, BTreeMap<String, ToolPin>>,
    pub artifacts: BTreeMap<String, ArtifactDigest>,
    pub inputs: BTreeMap<String, ArtifactDigest>,
}
#[derive(Debug, Clone, Copy)]
pub struct RecipePins<'a> {
    pub definitions: &'a DefinitionPins,
    pub source: &'a ResolvedPinFields,
    pub commands: &'a CommandPins,
    /// Direct checks use their declaration names for completeness diagnostics.
    pub checks: &'a BTreeMap<String, CheckDigest>,
    /// Additional context-bound policies required through the dependency closure.
    pub transitive_checks: &'a BTreeSet<CheckDigest>,
}
#[derive(Debug, thiserror::Error)]
pub enum PinGap {
    #[error("source has no acquired byte/tree identity")]
    Source,
    #[error("missing acquired tool identity for {package:?}/{command:?}")]
    Tool { package: String, command: String },
    #[error("missing acquired artifact identity for {0:?}")]
    Artifact(String),
    #[error("missing captured workspace input identity for {0:?}")]
    Input(String),
    #[error("missing required check policy identity for {0:?}")]
    Check(String),
    #[error("claimed tool digest does not match the acquired executable")]
    ToolMismatch,
    #[error("package identity was requested for the wrong producer kind")]
    ProducerKind,
    #[error(transparent)]
    Invalid(#[from] InvalidDigest),
}

pub fn production_digest(
    recipe: &RecipeOutput,
    pins: &RecipePins,
) -> Result<ProductionDigest, PinGap> {
    encode::production(recipe, pins).map(ProductionDigest)
}

impl ArtifactDigest {
    pub fn recipe_output(recipe: ProductionDigest) -> Self {
        let mut writer = encode::Encoder::new(b"gripsack/v6/recipe-output");
        writer.field(recipe.bytes());
        Self(writer.finish())
    }
    pub fn package_output(package: PackageProductionDigest) -> Self {
        let mut writer = encode::Encoder::new(b"gripsack/v6/package-output");
        writer.field(package.bytes());
        Self(writer.finish())
    }
}
pub fn recipe_digest(recipe: &RecipeOutput, pins: &RecipePins) -> Result<RecipeDigest, PinGap> {
    encode::recipe(recipe, pins).map(RecipeDigest)
}
pub fn check_digest(
    check: &CheckOutput,
    pins: &CommandPins,
    execution: &RecipeExecution,
) -> Result<CheckDigest, PinGap> {
    encode::check(check, pins, execution).map(CheckDigest)
}
pub fn recipe_package_digest(
    package: &PackageOutput,
    recipe: RecipeDigest,
    runtime: &BTreeMap<String, PackageDigest>,
) -> Result<PackageDigest, PinGap> {
    if !matches!(package.producer, super::WorkspaceProducer::Recipe { .. }) {
        return Err(PinGap::ProducerKind);
    }
    let mut writer = encode::Encoder::new(b"gripsack/v6/package/recipe");
    writer.field(recipe.bytes());
    encode::package(&mut writer, package, runtime)?;
    Ok(PackageDigest(writer.finish()))
}
pub fn provider_package_digest(
    package: &PackageOutput,
    resolved: &ResolvedPinFields,
    runtime: &BTreeMap<String, PackageDigest>,
) -> Result<PackageDigest, PinGap> {
    let mut writer = encode::Encoder::new(b"gripsack/v6/package/provider");
    let super::WorkspaceProducer::Provider { provider } = &package.producer else {
        return Err(PinGap::ProducerKind);
    };
    writer.source(provider);
    writer.resolved(resolved)?;
    encode::package(&mut writer, package, runtime)?;
    Ok(PackageDigest(writer.finish()))
}

pub fn recipe_package_production_digest(
    package: &PackageOutput,
    recipe: ProductionDigest,
    runtime: &BTreeMap<String, PackageProductionDigest>,
) -> Result<PackageProductionDigest, PinGap> {
    if !matches!(package.producer, super::WorkspaceProducer::Recipe { .. }) {
        return Err(PinGap::ProducerKind);
    }
    let mut writer = encode::Encoder::new(b"gripsack/v6/package-production/recipe");
    writer.field(recipe.bytes());
    encode::package(&mut writer, package, runtime)?;
    Ok(PackageProductionDigest(writer.finish()))
}
pub fn provider_package_production_digest(
    package: &PackageOutput,
    resolved: &ResolvedPinFields,
    runtime: &BTreeMap<String, PackageProductionDigest>,
) -> Result<PackageProductionDigest, PinGap> {
    let super::WorkspaceProducer::Provider { provider } = &package.producer else {
        return Err(PinGap::ProducerKind);
    };
    let mut writer = encode::Encoder::new(b"gripsack/v6/package-production/provider");
    writer.source(provider);
    writer.resolved(resolved)?;
    encode::package(&mut writer, package, runtime)?;
    Ok(PackageProductionDigest(writer.finish()))
}
pub fn lock_digest(lock: &WorkspaceLock) -> LockDigest {
    LockDigest(encode::lock(lock))
}
pub fn plan_digest(
    workspace: &WorkspaceCatalog,
    facts: &HostFacts,
    definitions: &DefinitionPins,
    packages: &BTreeMap<String, PackageDigest>,
    lock: Option<&WorkspaceLock>,
) -> PlanDigest {
    PlanDigest(encode::plan(workspace, facts, definitions, packages, lock))
}

/// The canonical identity of a frozen Conda closure (A3): a lock pin's
/// recorded `resolved.sha256` for Conda sources. Computed from the
/// complete records — never from artifact bytes — so the portable lock
/// reader can re-derive it without network or archives.
pub fn conda_closure_digest(environment: &super::lock::LockedCondaEnvironment) -> ArtifactDigest {
    ArtifactDigest(encode::conda_closure(environment))
}
