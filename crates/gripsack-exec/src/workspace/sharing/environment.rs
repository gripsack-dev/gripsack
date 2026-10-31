//! Home-bound passive composition of independently realized original inputs.
//! The root signature vouches for local members; independent manifests retain
//! their own signature authority. No installed prefix is copied into another.
use super::{
    closure::Closure,
    format::{self, PortableManifest, RuntimeLayout},
    receipt,
    runtime::{self, failure},
};
use crate::workspace::{
    artifact::{self, Artifact},
    conda::CondaRuntimeReceipt,
    prepare::PreparedSource,
    stage::validate_output_tree,
};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_fetch::FetchLimits;
use gripsack_ir::{
    FetchSpec, workspace::RecipeOutputKind, workspace_model::lock::ResolvedPinFields,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Constructed only after runtime admission of the complete signed closure.
pub(super) struct AdmittedEnvironment {
    pub manifest: PortableManifest,
    pub identity: String,
    pub artifact: Arc<Artifact>,
    pub components: BTreeMap<String, AdmittedMember>,
}
pub(super) enum AdmittedMember {
    Package(Arc<AdmittedPackage>),
    Environment(Arc<AdmittedEnvironment>),
}

/// Package realization and its owner-specific placement/target authority.
/// Sharing this variant keeps catalogs compact without copying the large Conda
/// receipt carried by PreparedSource for repeated external package edges.
pub(super) struct AdmittedPackage {
    pub source: PreparedSource,
    pub target: super::target::PortableTarget,
    pub fixed_prefix: Option<PathBuf>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EnvironmentReceipt {
    home: PathBuf,
    /// Exact manifest-local ordered selections and literal composition.
    compositions: BTreeMap<String, Composition>,
    /// Local members use manifest/id; independent package roots use manifest.
    members: BTreeMap<String, MemberReceipt>,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Composition {
    roots: Vec<String>,
    commands: BTreeMap<String, format::PortableCommand>,
    environment: BTreeMap<String, format::PortableEnvironmentValue>,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberReceipt {
    payload: PathBuf,
    tree: String,
    conda: Option<CondaRuntimeReceipt>,
}
fn compositions(closure: &Closure) -> BTreeMap<String, Composition> {
    closure
        .manifests
        .iter()
        .filter(|(_, manifest)| matches!(manifest.layout, RuntimeLayout::Environment {}))
        .map(|(identity, manifest)| {
            (
                identity.clone(),
                Composition {
                    roots: manifest.roots.clone(),
                    commands: manifest.commands.clone(),
                    environment: manifest.environment.clone(),
                },
            )
        })
        .collect()
}

fn independent_roots(closure: &Closure) -> BTreeSet<&str> {
    let mut roots = BTreeSet::new();
    for (identity, manifest) in &closure.manifests {
        if matches!(manifest.layout, RuntimeLayout::Environment {}) {
            roots.insert(identity.as_str());
            roots.extend(manifest.components.iter().filter_map(|component| {
                component
                    .external
                    .as_ref()
                    .map(|reference| reference.manifest.as_str())
            }));
        }
    }
    roots
}
fn evidence(artifact: &Artifact, conda: Option<CondaRuntimeReceipt>) -> MemberReceipt {
    MemberReceipt {
        payload: artifact.payload.clone(),
        tree: artifact.tree.to_string(),
        conda,
    }
}
fn publish_tree(
    ctx: &Ctx,
    source: &Path,
    tree: gripsack_store::hash::PayloadHash,
) -> Result<Artifact, ExecError> {
    let payload = gripsack_store::content_path(&ctx.home, "package-payload", tree.as_str());
    if artifact::retained_directory(&ctx.home, &payload)?.is_some() {
        if validate_output_tree(
            &payload,
            RecipeOutputKind::Tree,
            "portable member",
            ctx.fetch.limits(),
        )?
        .tree_hash()
            != &tree
        {
            return Err(failure(
                "retained environment member tree differs from original",
            ));
        }
    } else {
        crate::source::publish(ctx, "portable environment member", source, &payload)?;
    }
    Ok(Artifact {
        root: payload.clone(),
        payload: payload.clone(),
        tree,
        retention: BTreeSet::from([payload]),
    })
}

pub(super) fn import(
    ctx: &Ctx,
    session: &LifecycleSession,
    closure: &Closure,
    blobs: &Path,
    prepared: &super::conda::PreparedConda,
) -> Result<(Artifact, EnvironmentReceipt), ExecError> {
    let stage = tempfile::Builder::new()
        .prefix("environment-members-")
        .tempdir_in(&ctx.home)?;
    let mut record = EnvironmentReceipt {
        home: ctx.home.clone(),
        compositions: compositions(closure),
        members: BTreeMap::new(),
    };
    let mut retention = BTreeSet::new();
    let independent = independent_roots(closure);
    for (identity, manifest) in &closure.manifests {
        // Descendants of a relocatable aggregate are already reconstructed and
        // checked inside that aggregate. Do not create redundant standalone
        // copies beyond the complete closure's occurrence-based resource budget.
        if !independent.contains(identity.as_str()) {
            continue;
        }
        if matches!(manifest.layout, RuntimeLayout::Environment {}) {
            for component in &manifest.components {
                if component.external.is_some() {
                    continue;
                }
                let key = format!("{identity}/{}", component.id);
                let (artifact, conda) = if let Some(portable) = &component.conda {
                    let value = super::conda::import(
                        ctx,
                        session,
                        &component.id,
                        portable,
                        blobs,
                        prepared,
                    )?;
                    (value.artifact, Some(value.receipt))
                } else {
                    let destination = stage.path().join(identity).join(&component.id);
                    std::fs::create_dir_all(
                        destination
                            .parent()
                            .ok_or_else(|| failure("member parent missing"))?,
                    )?;
                    let blob = component
                        .payload
                        .as_ref()
                        .ok_or_else(|| failure("native environment member blob missing"))?;
                    let original = blobs.join(&blob.sha256);
                    let archive = blobs.join("component.tar");
                    std::fs::rename(&original, &archive)?;
                    ctx.fetch.fetch(
                        &FetchSpec::File {
                            path: archive
                                .to_str()
                                .ok_or_else(|| failure("archive path is not UTF-8"))?
                                .into(),
                        },
                        &destination,
                        None,
                    )?;
                    std::fs::rename(&archive, &original)?;
                    runtime::restore_directory_modes(&destination, component)?;
                    receipt::inventory(&destination, component)?;
                    let tree = validate_output_tree(
                        &destination,
                        RecipeOutputKind::Tree,
                        "portable native member",
                        ctx.fetch.limits(),
                    )?
                    .tree_hash()
                    .clone();
                    if component.tree.as_deref() != Some(tree.as_str()) {
                        return Err(failure(
                            "native environment member differs from signed tree",
                        ));
                    }
                    (publish_tree(ctx, &destination, tree)?, None)
                };
                record.members.insert(key, evidence(&artifact, conda));
                retention.extend(artifact.retention);
            }
        } else {
            let (artifact, conda) = if matches!(manifest.layout, RuntimeLayout::Conda) {
                let component = &manifest.components[0];
                let value = super::conda::import(
                    ctx,
                    session,
                    &component.id,
                    component
                        .conda
                        .as_ref()
                        .ok_or_else(|| failure("Conda archive set missing"))?,
                    blobs,
                    prepared,
                )?;
                runtime::check_payload(
                    manifest,
                    closure,
                    &value.artifact.payload,
                    ctx.fetch.limits(),
                )?;
                (value.artifact, Some(value.receipt))
            } else {
                let destination = stage.path().join(identity);
                runtime::materialize_tree(ctx, closure, manifest, blobs, &destination)?;
                let tree =
                    runtime::check_payload(manifest, closure, &destination, ctx.fetch.limits())?;
                if let RuntimeLayout::Fixed { prefix, .. } = &manifest.layout
                    && Path::new(prefix)
                        != gripsack_store::content_path(&ctx.home, "package-payload", tree.as_str())
                {
                    return Err(failure(
                        "fixed environment member differs from declared exact destination",
                    ));
                }
                (publish_tree(ctx, &destination, tree)?, None)
            };
            record
                .members
                .insert(identity.clone(), evidence(&artifact, conda));
            retention.extend(artifact.retention);
        }
    }
    let metadata = stage.path().join("metadata");
    std::fs::create_dir(&metadata)?;
    std::fs::write(
        metadata.join("environment.json"),
        receipt::encode_record(&record)?,
    )?;
    let tree = validate_output_tree(
        &metadata,
        RecipeOutputKind::Tree,
        "portable environment",
        ctx.fetch.limits(),
    )?
    .tree_hash()
    .clone();
    let payload = gripsack_store::content_path(&ctx.home, "package-environment", tree.as_str());
    if artifact::retained_directory(&ctx.home, &payload)?.is_none() {
        crate::source::publish(ctx, "portable environment", &metadata, &payload)?;
    }
    retention.insert(payload.clone());
    Ok((
        Artifact {
            root: payload.clone(),
            payload,
            tree,
            retention,
        },
        record,
    ))
}

/// A member is vouched for locally or independently published, never both.
#[derive(Clone, Copy)]
enum MemberAuthority<'a> {
    Local(&'a format::Component),
    Independent(&'a PortableManifest),
}

fn inspect_member(
    home: &Path,
    key: &str,
    record: &MemberReceipt,
    authority: MemberAuthority<'_>,
    closure: &Closure,
    limits: FetchLimits,
    prepared: &super::conda::PreparedConda,
) -> Result<PreparedSource, ExecError> {
    let portable = match authority {
        MemberAuthority::Local(component) => component.conda.as_ref(),
        MemberAuthority::Independent(manifest)
            if matches!(manifest.layout, RuntimeLayout::Conda) =>
        {
            manifest.components[0].conda.as_ref()
        }
        MemberAuthority::Independent(_) => None,
    };
    let (artifact, conda) = if let Some(portable) = portable {
        let value = super::conda::inspect_retained(key, home, portable, prepared)?;
        if record.conda.as_ref() != Some(&value.receipt) {
            return Err(failure("environment member Conda receipt mismatch"));
        }
        (value.artifact, Some(value.receipt))
    } else {
        if record.conda.is_some() {
            return Err(failure("native member has unexpected Conda receipt"));
        }
        format::digest(&record.tree)?;
        let payload = gripsack_store::content_path(home, "package-payload", &record.tree);
        if let MemberAuthority::Independent(PortableManifest {
            layout: RuntimeLayout::Fixed { prefix, .. },
            ..
        }) = authority
            && Path::new(prefix) != payload
        {
            return Err(failure(
                "fixed member prefix differs from actual destination",
            ));
        }
        if artifact::retained_directory(home, &payload)?.is_none() {
            return Err(failure("environment member payload missing"));
        }
        let tree =
            validate_output_tree(&payload, RecipeOutputKind::Tree, "portable member", limits)?
                .tree_hash()
                .clone();
        if let MemberAuthority::Local(component) = authority {
            receipt::inventory(&payload, component)?;
            if component.tree.as_deref() != Some(tree.as_str()) {
                return Err(failure("native member differs from signed tree"));
            }
        }
        (
            Artifact {
                root: payload.clone(),
                payload: payload.clone(),
                tree,
                retention: BTreeSet::from([payload]),
            },
            None,
        )
    };
    if artifact.payload != record.payload || artifact.tree.as_str() != record.tree {
        return Err(failure(
            "environment member realization differs from receipt",
        ));
    }
    if let MemberAuthority::Independent(manifest) = authority {
        runtime::check_payload(manifest, closure, &artifact.payload, limits)?;
    }
    Ok(PreparedSource {
        resolved: ResolvedPinFields {
            sha256: None,
            tree256: Some(artifact.tree.to_string()),
            ..Default::default()
        },
        artifact: Arc::new(artifact),
        conda,
    })
}

pub(super) fn inspect(
    home: &Path,
    closure: &Closure,
    record: &EnvironmentReceipt,
    realized_tree: &str,
    limits: FetchLimits,
    import_root: &Path,
    prepared: &super::conda::PreparedConda,
) -> Result<Arc<AdmittedEnvironment>, ExecError> {
    if record.home != home || record.compositions != compositions(closure) {
        return Err(failure(
            "environment composition/home differs from admitted closure",
        ));
    }
    let mut sources = BTreeMap::new();
    let mut retention = BTreeSet::from([import_root.to_path_buf()]);
    let independent = independent_roots(closure);
    for (identity, manifest) in &closure.manifests {
        if !independent.contains(identity.as_str()) {
            continue;
        }
        if matches!(manifest.layout, RuntimeLayout::Environment {}) {
            for component in &manifest.components {
                if component.external.is_some() {
                    continue;
                }
                let key = format!("{identity}/{}", component.id);
                let evidence = record
                    .members
                    .get(&key)
                    .ok_or_else(|| failure("environment member receipt missing"))?;
                let source = inspect_member(
                    home,
                    &key,
                    evidence,
                    MemberAuthority::Local(component),
                    closure,
                    limits,
                    prepared,
                )?;
                retention.extend(source.artifact.retention.iter().cloned());
                sources.insert(key, source);
            }
        } else {
            let evidence = record
                .members
                .get(identity)
                .ok_or_else(|| failure("external package receipt missing"))?;
            let source = inspect_member(
                home,
                identity,
                evidence,
                MemberAuthority::Independent(manifest),
                closure,
                limits,
                prepared,
            )?;
            retention.extend(source.artifact.retention.iter().cloned());
            sources.insert(identity.clone(), source);
        }
    }
    if sources.len() != record.members.len() {
        return Err(failure("environment receipt has unreferenced members"));
    }
    let bytes = receipt::encode_record(record)?;
    // Receipt paths are never placement authority. Check the canonical store
    // destination and its complete contents against independently checked data.
    format::digest(realized_tree)?;
    let payload = gripsack_store::content_path(home, "package-environment", realized_tree);
    let directory = artifact::retained_directory(home, &payload)?
        .ok_or_else(|| failure("environment metadata missing"))?;
    if receipt::read(
        &directory,
        Path::new("environment.json"),
        receipt::MAX_MATERIALIZATION_RECORD_BYTES as u64,
    )? != bytes
    {
        return Err(failure("environment metadata differs from receipt"));
    }
    let tree = validate_output_tree(
        &payload,
        RecipeOutputKind::Tree,
        "portable environment",
        limits,
    )?
    .tree_hash()
    .clone();
    if tree.as_str() != realized_tree {
        return Err(failure("environment metadata tree mismatch"));
    }
    retention.insert(payload.clone());
    let artifact = Arc::new(Artifact {
        root: payload.clone(),
        payload,
        tree,
        retention,
    });
    // This memo belongs to this admitted root and its retention authority only.
    catalog(
        &closure.root,
        closure,
        &mut sources,
        &artifact,
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
    )
}

fn catalog(
    identity: &str,
    closure: &Closure,
    sources: &mut BTreeMap<String, PreparedSource>,
    artifact: &Arc<Artifact>,
    memo: &mut BTreeMap<String, Arc<AdmittedEnvironment>>,
    packages: &mut BTreeMap<String, Arc<AdmittedPackage>>,
) -> Result<Arc<AdmittedEnvironment>, ExecError> {
    if let Some(environment) = memo.get(identity) {
        return Ok(Arc::clone(environment));
    }
    let manifest = &closure.manifests[identity];
    let mut components = BTreeMap::new();
    for component in &manifest.components {
        let key = component
            .external
            .as_ref()
            .map(|reference| reference.manifest.clone())
            .unwrap_or_else(|| format!("{identity}/{}", component.id));
        let member = if component.external.is_some()
            && matches!(
                closure.manifests[&key].layout,
                RuntimeLayout::Environment {}
            ) {
            AdmittedMember::Environment(catalog(&key, closure, sources, artifact, memo, packages)?)
        } else if let Some(package) = packages.get(&key) {
            AdmittedMember::Package(Arc::clone(package))
        } else {
            let mut source = sources
                .remove(&key)
                .ok_or_else(|| failure("admitted environment member absent"))?;
            // Root receipt retention is authority for local as well as external
            // members; a leaf cannot escape current-policy closure readmission.
            let mut retention = source.artifact.retention.clone();
            retention.extend(artifact.retention.iter().cloned());
            let owner = component
                .external
                .as_ref()
                .map(|reference| &closure.manifests[&reference.manifest])
                .unwrap_or(manifest);
            let fixed_prefix = match &owner.layout {
                RuntimeLayout::Fixed { prefix, .. } => Some(PathBuf::from(prefix)),
                _ => None,
            };
            source.artifact = Arc::new(Artifact {
                root: source.artifact.root.clone(),
                payload: source.artifact.payload.clone(),
                tree: source.artifact.tree.clone(),
                retention,
            });
            let package = Arc::new(AdmittedPackage {
                source,
                target: owner.target.clone(),
                fixed_prefix,
            });
            packages.insert(key, Arc::clone(&package));
            AdmittedMember::Package(package)
        };
        components.insert(component.id.clone(), member);
    }
    let value = AdmittedEnvironment {
        manifest: manifest.clone(),
        identity: identity.into(),
        artifact: Arc::clone(artifact),
        components,
    };
    for command in value.manifest.commands.values() {
        let member = value
            .components
            .get(&command.component)
            .ok_or_else(|| failure("command component missing"))?;
        let path = member_path(member, &command.executable)?;
        let metadata = std::fs::metadata(path)?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err(failure("environment command is not executable"));
        }
    }
    for license in &value.manifest.licenses {
        if !environment_path(&value, license)?.is_file() {
            return Err(failure("environment license is absent"));
        }
    }
    let value = Arc::new(value);
    memo.insert(identity.into(), Arc::clone(&value));
    Ok(value)
}
fn environment_path(
    environment: &AdmittedEnvironment,
    relative: &str,
) -> Result<PathBuf, ExecError> {
    if let Some(alias) = relative.strip_prefix("bin/") {
        let command = environment
            .manifest
            .commands
            .get(alias)
            .ok_or_else(|| failure("nested environment command missing"))?;
        return member_path(
            environment
                .components
                .get(&command.component)
                .ok_or_else(|| failure("nested command component missing"))?,
            &command.executable,
        );
    }
    let (id, path) = relative
        .strip_prefix("components/")
        .and_then(|path| path.split_once('/'))
        .ok_or_else(|| failure("nested environment path is not a command or component path"))?;
    member_path(
        environment
            .components
            .get(id)
            .ok_or_else(|| failure("nested environment member missing"))?,
        path,
    )
}
fn member_path(member: &AdmittedMember, relative: &str) -> Result<PathBuf, ExecError> {
    match member {
        AdmittedMember::Environment(environment) => environment_path(environment, relative),
        AdmittedMember::Package(package) => {
            let root = package.source.artifact.payload.canonicalize()?;
            let path = root.join(relative).canonicalize()?;
            if !path.starts_with(root) {
                return Err(failure("environment member path escapes its actual root"));
            }
            Ok(path)
        }
    }
}

impl AdmittedEnvironment {
    pub(super) fn admits_fixed(&self, payload: &Path) -> bool {
        let mut visited = BTreeSet::new();
        let mut pending = vec![self];
        while let Some(environment) = pending.pop() {
            if !visited.insert(environment.identity.as_str()) {
                continue;
            }
            for member in environment.components.values() {
                match member {
                    AdmittedMember::Package(package) => {
                        if package.fixed_prefix.as_deref() == Some(payload)
                            && package.source.artifact.payload == payload
                        {
                            return true;
                        }
                    }
                    AdmittedMember::Environment(child) => pending.push(child),
                }
            }
        }
        false
    }
}
