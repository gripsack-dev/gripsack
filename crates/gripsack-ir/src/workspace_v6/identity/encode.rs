mod commands;
mod declarations;
use super::{CheckDigest, CommandPins, PackageDigest, PinGap, RecipePins};
use crate::workspace::{
    PlatformAbi, PlatformArch, PlatformOs, RecipeOutputKind, WorkspacePlatform,
};
use crate::workspace_v6::lock::{DefinitionPins, ResolvedPinFields, WorkspaceLock};
use crate::workspace_v6::{
    CheckOutput, LockedSource, PackageLayoutV6, PackageOutput, RecipeExecution, RecipeOutput,
    WorkspaceSourceV6, WorkspaceV6,
};
use crate::{FetchSpec, HostFacts};
use gripsack_policy::semantic::length_header;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Every field is length-delimited. Domain separation prevents archive/tree,
/// recipe/package, lock and captured-plan identities from being interchangeable.
pub(super) struct Encoder(Sha256);
impl Encoder {
    pub(super) fn new(domain: &[u8]) -> Self {
        let mut writer = Self(Sha256::new());
        writer.field(domain);
        writer
    }
    /// Stream one length-delimited field: the verified kernel supplies
    /// the exact header bytes (semantic::length_header == le64), the
    /// payload flows through uncopied.
    pub(super) fn field(&mut self, bytes: &[u8]) {
        self.0.update(length_header(bytes.len() as u64));
        self.0.update(bytes);
    }
    fn text(&mut self, text: &str) {
        self.field(text.as_bytes());
    }
    fn number(&mut self, value: u64) {
        self.field(&value.to_le_bytes());
    }
    fn optional(&mut self, value: &Option<String>) {
        match value {
            Some(value) => {
                self.field(b"some");
                self.text(value);
            }
            None => self.field(b"none"),
        }
    }
    pub(super) fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
    fn definitions(&mut self, pins: &DefinitionPins) {
        self.field(pins.frontend.bytes());
        self.number(pins.imports.len() as u64);
        for (source, pin) in &pins.imports {
            self.text(source);
            self.field(pin.bytes());
        }
    }
    pub(super) fn resolved(&mut self, pin: &ResolvedPinFields) -> Result<(), PinGap> {
        if pin.sha256.is_none() && pin.tree256.is_none() {
            return Err(PinGap::Source);
        }
        self.optional(&pin.url);
        self.optional(&pin.version);
        self.optional(&pin.api_url);
        for digest in [&pin.sha256, &pin.tree256, &pin.repo256] {
            match digest {
                Some(value) => {
                    self.field(b"some");
                    self.field(super::ArtifactDigest::parse(value)?.bytes());
                }
                None => self.field(b"none"),
            }
        }
        Ok(())
    }
    pub(super) fn fetch(&mut self, source: &FetchSpec) {
        match source {
            FetchSpec::GithubRelease {
                repo,
                asset,
                version,
                sha256,
                base_url,
            } => {
                self.field(b"github_release");
                self.text(repo);
                self.text(asset);
                self.optional(version);
                self.optional(sha256);
                self.optional(base_url);
            }
            FetchSpec::Tarball {
                url,
                sha256,
                api_url,
            } => {
                self.field(b"tarball");
                self.text(url);
                self.optional(sha256);
                self.optional(api_url);
            }
            FetchSpec::Git { url, rev } => {
                self.field(b"git");
                self.text(url);
                self.optional(rev);
            }
            FetchSpec::File { path } => {
                self.field(b"file");
                self.text(path);
            }
            FetchSpec::Plugin { name, args } => {
                self.field(b"plugin");
                self.text(name);
                self.json(args);
            }
            FetchSpec::Brew {
                formula,
                version,
                sha256,
            } => {
                self.field(b"brew");
                self.text(formula);
                self.optional(version);
                self.optional(sha256);
            }
            FetchSpec::Pixi {
                package,
                version,
                sha256,
            } => {
                self.field(b"pixi");
                self.text(package);
                self.optional(version);
                self.optional(sha256);
            }
        }
    }
    fn json(&mut self, value: &serde_json::Value) {
        use serde_json::Value;
        match value {
            Value::Null => self.field(b"null"),
            Value::Bool(value) => {
                self.field(b"bool");
                self.field(&[u8::from(*value)]);
            }
            Value::Number(value) => {
                self.field(b"number");
                self.text(&value.to_string());
            }
            Value::String(value) => {
                self.field(b"string");
                self.text(value);
            }
            Value::Array(values) => {
                self.field(b"array");
                self.number(values.len() as u64);
                for value in values {
                    self.json(value);
                }
            }
            Value::Object(values) => {
                self.field(b"object");
                self.number(values.len() as u64);
                // Explicit sorting also preserves identity if another workspace
                // dependency enables serde_json's preserve_order feature.
                let mut entries: Vec<_> = values.iter().collect();
                entries.sort_unstable_by_key(|(key, _)| *key);
                for (key, value) in entries {
                    self.text(key);
                    self.json(value);
                }
            }
        }
    }
    fn platform(&mut self, target: &WorkspacePlatform) {
        self.field(match target.os {
            PlatformOs::Linux => b"linux",
            PlatformOs::Macos => b"macos",
        });
        self.field(match target.arch {
            PlatformArch::X86_64 => b"x86_64",
            PlatformArch::Aarch64 => b"aarch64",
        });
        self.field(match target.abi {
            None => b"unspecified",
            Some(PlatformAbi::Gnu) => b"gnu",
            Some(PlatformAbi::Musl) => b"musl",
            Some(PlatformAbi::Darwin) => b"darwin",
        });
        match &target.minimum_os {
            None => self.field(b"no-floor"),
            Some(version) => {
                self.field(b"minimum-os");
                self.number(version.major.into());
                self.number(version.minor.into());
                self.number(version.patch.unwrap_or(0).into());
            }
        }
    }
    fn execution(&mut self, execution: &RecipeExecution) {
        match execution {
            RecipeExecution::Host { .. } => self.field(b"host/unconfined"),
            RecipeExecution::IsolatedLinux {
                platform,
                toolchain,
                ..
            } => {
                self.field(b"isolated-linux/network-none/readonly-inputs/v1");
                self.platform(platform);
                self.text(&toolchain.reference);
            }
        }
    }
    fn layout(&mut self, layout: &PackageLayoutV6) {
        match layout {
            PackageLayoutV6::Relocatable => self.field(b"relocatable"),
            PackageLayoutV6::FixedPrefix { prefix } => {
                self.field(b"fixed-prefix");
                self.text(prefix.as_str());
            }
            PackageLayoutV6::PrefixMaterialized => self.field(b"prefix-materialized"),
        }
    }
    /// The declared v6 acquisition source: variant tag plus every
    /// semantic field. Channel order is priority order and stays
    /// declared-ordered; declaration locations never enter identity.
    pub(super) fn source(&mut self, source: &WorkspaceSourceV6) {
        match source {
            WorkspaceSourceV6::Fetch(fetch) => {
                self.field(b"fetch");
                self.fetch(&fetch.fetch);
            }
            WorkspaceSourceV6::CondaEnvironment(source) => {
                self.field(b"conda-environment");
                self.number(source.channels.len() as u64);
                for channel in &source.channels {
                    self.text(channel);
                }
                self.number(source.packages.len() as u64);
                for (name, spec) in &source.packages {
                    self.text(name);
                    self.text(spec);
                }
                self.number(source.platforms.len() as u64);
                for platform in &source.platforms {
                    self.platform(platform);
                }
            }
            WorkspaceSourceV6::PixiLock(source) => {
                self.field(b"pixi-lock");
                self.text(&source.manifest);
                self.text(&source.lock);
                self.text(&source.environment);
            }
        }
    }
    /// The location-free locked source — identical semantic fold as
    /// `source`, so a lock rewrite never shifts identities.
    pub(super) fn locked_source(&mut self, source: &LockedSource) {
        match source {
            LockedSource::Fetch { fetch } => {
                self.field(b"fetch");
                self.fetch(fetch);
            }
            LockedSource::CondaEnvironment(source) => {
                self.field(b"conda-environment");
                self.number(source.channels.len() as u64);
                for channel in &source.channels {
                    self.text(channel);
                }
                self.number(source.packages.len() as u64);
                for (name, spec) in &source.packages {
                    self.text(name);
                    self.text(spec);
                }
                self.number(source.platforms.len() as u64);
                for platform in &source.platforms {
                    self.platform(platform);
                }
            }
            LockedSource::PixiLock(source) => {
                self.field(b"pixi-lock");
                self.text(&source.manifest);
                self.text(&source.lock);
                self.text(&source.environment);
                self.optional(&source.manifest_sha256);
                self.optional(&source.lock_sha256);
            }
        }
    }
}

pub(super) fn production(recipe: &RecipeOutput, pins: &RecipePins) -> Result<[u8; 32], PinGap> {
    let mut writer = Encoder::new(b"gripsack/v6/production");
    writer.definitions(pins.definitions);
    writer.source(&recipe.source);
    writer.resolved(pins.source)?;
    writer.execution(&recipe.execution);
    writer.platform(&recipe.target);
    writer.field(match recipe.output_kind {
        RecipeOutputKind::File => b"file",
        RecipeOutputKind::Tree => b"tree",
    });
    writer.number(recipe.steps.len() as u64);
    for step in &recipe.steps {
        commands::step(&mut writer, step, pins.commands)?;
    }
    Ok(writer.finish())
}
pub(super) fn recipe(recipe: &RecipeOutput, pins: &RecipePins) -> Result<[u8; 32], PinGap> {
    let production = production(recipe, pins)?;
    for name in &recipe.checks {
        if !pins.checks.contains_key(name) {
            return Err(PinGap::Check(name.clone()));
        }
    }
    let mut writer = Encoder::new(b"gripsack/v6/validated-recipe");
    writer.field(&production);
    let checks: BTreeSet<CheckDigest> = pins
        .checks
        .values()
        .chain(pins.transitive_checks)
        .copied()
        .collect();
    writer.number(checks.len() as u64);
    for check in checks {
        writer.field(check.bytes());
    }
    Ok(writer.finish())
}
pub(super) fn check(
    check: &CheckOutput,
    pins: &CommandPins,
    execution: &RecipeExecution,
) -> Result<[u8; 32], PinGap> {
    let mut writer = Encoder::new(b"gripsack/v6/required-check");
    writer.execution(execution);
    writer.field(
        pins.artifacts
            .get(&check.subject)
            .ok_or_else(|| PinGap::Artifact(check.subject.clone()))?
            .bytes(),
    );
    commands::command(&mut writer, &check.run, pins)?;
    Ok(writer.finish())
}
pub(super) fn package<D: AsRef<[u8; 32]>>(
    writer: &mut Encoder,
    package: &PackageOutput,
    runtime: &BTreeMap<String, D>,
) -> Result<(), PinGap> {
    writer.platform(&package.target);
    writer.layout(&package.layout);
    writer.number(package.commands.len() as u64);
    for (name, path) in &package.commands {
        writer.text(name);
        writer.text(path);
    }
    let mut closure = BTreeSet::new();
    for name in &package.runtime {
        closure.insert(
            runtime
                .get(name)
                .ok_or_else(|| PinGap::Artifact(name.clone()))?
                .as_ref(),
        );
    }
    writer.number(closure.len() as u64);
    for dependency in closure {
        writer.field(dependency);
    }
    Ok(())
}
pub(super) fn lock(lock: &WorkspaceLock) -> [u8; 32] {
    declarations::lock(lock)
}
pub(super) fn conda_closure(
    environment: &crate::workspace_v6::lock::LockedCondaEnvironment,
) -> [u8; 32] {
    declarations::conda_closure(environment)
}
pub(super) fn plan(
    workspace: &WorkspaceV6,
    facts: &HostFacts,
    definitions: &DefinitionPins,
    packages: &BTreeMap<String, PackageDigest>,
    lock: Option<&WorkspaceLock>,
) -> [u8; 32] {
    declarations::plan(workspace, facts, definitions, packages, lock)
}
