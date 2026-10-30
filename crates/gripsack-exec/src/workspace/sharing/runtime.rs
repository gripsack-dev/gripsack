//! Passive package import and current-policy admission over the common store.
use super::{
    closure::Closure,
    format::{self, PortableManifest, RuntimeLayout},
    receipt::{self, Receipt},
    trust::{TrustPolicy, VerifiedSignature},
};
use crate::workspace::{
    artifact::{self, Artifact},
    prepare::PreparedSource,
    roots::{self, RetentionSet, RootId},
    stage::validate_output_tree,
};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_fetch::FetchLimits;
use gripsack_ir::{
    FetchSpec, workspace::RecipeOutputKind, workspace_model::lock::ResolvedPinFields,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, Serialize)]
pub struct PackageFacts {
    pub manifest: String,
    pub publisher: String,
    pub namespace: String,
    pub trust_revision: String,
    pub key_id: String,
    pub layout: String,
    pub target: super::target::PortableTarget,
    /// Independently signed manifests only. Local aggregate members remain
    /// claims of the root publisher, not invented upstream identities.
    pub external: Vec<ExternalPackageFacts>,
    pub realized_tree: String,
    pub materialization: Option<String>,
    pub payload_digests: Vec<String>,
    pub payload: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalPackageFacts {
    pub manifest: String,
    pub namespace: String,
    pub publisher: String,
    pub key_id: String,
    pub trust_revision: String,
    pub target: super::target::PortableTarget,
    pub layout: String,
}

pub(super) struct AdmittedSource {
    pub prepared: PreparedSource,
    pub environment: Option<Arc<super::environment::AdmittedEnvironment>>,
}

struct LoadedImport {
    facts: PackageFacts,
    source: AdmittedSource,
}

pub(super) fn failure(detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        output: "package".into(),
        step: "portable import".into(),
        detail: detail.into(),
    }
}
fn imported_root(home: &Path, manifest: &str) -> Result<PathBuf, ExecError> {
    format::digest(manifest)?;
    Ok(gripsack_store::content_path(
        home,
        "package-import",
        manifest,
    ))
}
fn admit_closure(
    home: &Path,
    closure: &Closure,
) -> Result<
    (
        VerifiedSignature,
        BTreeMap<String, receipt::ExternalReceipt>,
    ),
    ExecError,
> {
    let mut root = None;
    let mut external = BTreeMap::new();
    let policy = TrustPolicy::load(home).map_err(|error| failure(error.to_string()))?;
    for (identity, object) in &closure.objects {
        let manifest = &closure.manifests[identity];
        let verified = signed(&policy, manifest, &object.manifest, &object.envelope)?;
        authorize(&verified, manifest)?;
        if identity == &closure.root {
            root = Some(verified);
        } else {
            external.insert(
                identity.clone(),
                receipt::ExternalReceipt {
                    trust_revision: verified.revision().into(),
                    key_id: verified.key_id().into(),
                },
            );
        }
    }
    for manifest in closure.manifests.values() {
        if !matches!(manifest.layout, RuntimeLayout::Environment {}) {
            for reference in manifest
                .components
                .iter()
                .filter_map(|component| component.external.as_ref())
            {
                if !matches!(
                    closure.manifests[&reference.manifest].layout,
                    RuntimeLayout::Relocatable { .. }
                ) {
                    return Err(failure(
                        "embedded aggregate members require relocatable layouts; use an environment for independent prefixes",
                    ));
                }
            }
        }
        for command in manifest.commands.values() {
            let reference = manifest
                .components
                .iter()
                .find(|component| component.id == command.component)
                .and_then(|component| component.external.as_ref());
            let Some(reference) = reference else {
                continue;
            };
            let child = &closure.manifests[&reference.manifest];
            if matches!(child.layout, RuntimeLayout::Environment {})
                && !command
                    .executable
                    .strip_prefix("bin/")
                    .is_some_and(|alias| child.commands.contains_key(alias))
            {
                return Err(failure(
                    "nested environment commands must select a signed bin/<alias> export",
                ));
            }
        }
    }
    Ok((
        root.ok_or_else(|| failure("closure root missing"))?,
        external,
    ))
}
fn signed(
    policy: &TrustPolicy,
    parsed: &PortableManifest,
    manifest: &[u8],
    envelope: &[u8],
) -> Result<VerifiedSignature, ExecError> {
    parsed.target.admit()?;
    let verified = policy
        .verify(manifest, envelope)
        .map_err(|error| failure(error.to_string()))?;
    verified
        .authorize(&parsed.namespace, &parsed.publisher)
        .map_err(|error| failure(error.to_string()))?;
    verified
        .authorize_requested_layouts(&[parsed.layout.name()])
        .map_err(|error| failure(error.to_string()))?;
    Ok(verified)
}
fn authorize(verified: &VerifiedSignature, manifest: &PortableManifest) -> Result<(), ExecError> {
    verified
        .authorize_layouts(
            &[manifest.layout.name()],
            &["manifest-integrity", "payload-integrity", "layout-safety"],
        )
        .map_err(|error| failure(error.to_string()))
}
pub(super) fn check_payload(
    manifest: &PortableManifest,
    closure: &Closure,
    payload: &Path,
    limits: FetchLimits,
) -> Result<gripsack_store::hash::PayloadHash, ExecError> {
    let checked =
        validate_output_tree(payload, RecipeOutputKind::Tree, "portable package", limits)?;
    check_payload_contents(manifest, closure, payload)?;
    Ok(checked.tree_hash().clone())
}

fn check_payload_contents(
    manifest: &PortableManifest,
    closure: &Closure,
    payload: &Path,
) -> Result<(), ExecError> {
    if !matches!(manifest.layout, RuntimeLayout::Conda) {
        let components = payload.join("components");
        let expected: BTreeSet<_> = manifest
            .components
            .iter()
            .map(|component| component.id.as_str())
            .collect();
        let mut actual = BTreeSet::new();
        for entry in std::fs::read_dir(payload)? {
            if entry?.file_name() != "components" {
                return Err(failure("unexpected aggregate root member"));
            }
        }
        for entry in std::fs::read_dir(&components)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                return Err(failure("aggregate component is not a directory"));
            }
            actual.insert(
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| failure("non UTF-8 component"))?,
            );
        }
        if actual.iter().map(String::as_str).collect::<BTreeSet<_>>() != expected {
            return Err(failure("aggregate component inventory mismatch"));
        }
        for component in &manifest.components {
            let root = components.join(&component.id);
            if let Some(reference) = &component.external {
                check_payload_contents(&closure.manifests[&reference.manifest], closure, &root)?;
                continue;
            }
            receipt::inventory(&root, component)?;
            if gripsack_store::canonical_tree_hash(&root)?.as_str()
                != component
                    .tree
                    .as_deref()
                    .ok_or_else(|| failure("component tree missing"))?
            {
                return Err(failure(
                    "portable component tree differs from signed identity",
                ));
            }
        }
    }
    let canonical = payload.canonicalize()?;
    for command in manifest.commands.values() {
        let path = if matches!(manifest.layout, RuntimeLayout::Conda) {
            payload.join(&command.executable)
        } else {
            payload
                .join("components")
                .join(&command.component)
                .join(&command.executable)
        };
        let selected = path.canonicalize()?;
        let metadata = std::fs::metadata(&selected)?;
        if !selected.starts_with(&canonical)
            || !metadata.is_file()
            || metadata.permissions().mode() & 0o111 == 0
        {
            return Err(failure(
                "portable command is not an executable inside the runtime closure",
            ));
        }
    }
    for license in &manifest.licenses {
        let relative = if matches!(manifest.layout, RuntimeLayout::Conda) {
            license
                .strip_prefix(&format!("components/{}/", manifest.components[0].id))
                .ok_or_else(|| failure("Conda license is not component-relative"))?
        } else {
            license.as_str()
        };
        let path = payload.join(relative).canonicalize()?;
        if !path.starts_with(&canonical) || !std::fs::metadata(path)?.is_file() {
            return Err(failure("license is absent from runtime payload"));
        }
    }
    Ok(())
}

/// Generic extraction uses traversable directory defaults. The signed inventory
/// is the final mode authority; restore it through opened no-follow handles,
/// deepest paths first, before exact inventory and tree identity validation.
pub(super) fn restore_directory_modes(
    root: &Path,
    component: &format::Component,
) -> Result<(), ExecError> {
    let directory = gripsack_fs::open(root)?;
    for entry in component.inventory.iter().rev() {
        if matches!(entry.kind, format::EntryKind::Directory) {
            let child = gripsack_fs::open_dir_nofollow(&directory, Path::new(&entry.path))?;
            child
                .into_std_file()
                .set_permissions(std::fs::Permissions::from_mode(entry.mode))?;
        }
    }
    Ok(())
}

pub(super) fn materialize_tree(
    ctx: &Ctx,
    closure: &Closure,
    manifest: &PortableManifest,
    blobs: &Path,
    aggregate: &Path,
) -> Result<(), ExecError> {
    std::fs::create_dir_all(aggregate.join("components"))?;
    for component in &manifest.components {
        let component_root = aggregate.join("components").join(&component.id);
        if let Some(reference) = &component.external {
            // Reconstruct from retained originals at the child's qualified root.
            // Never copy a realized fixed/Conda prefix or share mutable hardlinks.
            materialize_tree(
                ctx,
                closure,
                &closure.manifests[&reference.manifest],
                blobs,
                &component_root,
            )?;
            continue;
        }
        let blob = component
            .payload
            .as_ref()
            .ok_or_else(|| failure("tree component missing blob"))?;
        let original = blobs.join(&blob.sha256);
        // Only the private staging copy is renamed. A repeated DAG member uses
        // the same immutable original after the preceding extraction finishes.
        let archive = blobs.join("component.tar");
        std::fs::rename(&original, &archive)?;
        ctx.fetch.fetch(
            &FetchSpec::File {
                path: archive
                    .to_str()
                    .ok_or_else(|| failure("bundle path is not UTF-8"))?
                    .into(),
            },
            &component_root,
            None,
        )?;
        std::fs::rename(&archive, &original)?;
        restore_directory_modes(&component_root, component)?;
        receipt::inventory(&component_root, component)?;
    }
    Ok(())
}

pub fn import_bundle(
    ctx: &Ctx,
    session: &LifecycleSession,
    bundle: &Path,
) -> Result<PackageFacts, ExecError> {
    artifact::authority(ctx, session)?;
    let source = gripsack_fs::open(bundle)?;
    let closure = Closure::read(&source, ctx.fetch.limits())?;
    let (verified, external) = admit_closure(&ctx.home, &closure)?;
    let identity = closure.root.clone();
    let manifest = &closure.manifests[&identity];
    let destination = imported_root(&ctx.home, &identity)?;
    if artifact::retained_directory(&ctx.home, &destination)?.is_some() {
        // Even a warm receipt cannot hide a corrupt incoming closure object.
        let incoming = gripsack_fs::open_dir_nofollow(&source, Path::new("blobs"))?;
        for descriptor in closure.blobs.values() {
            receipt::blob(&incoming, descriptor, None)?;
        }
        drop(closure.preflight_conda(
            &ctx.home,
            Some(&bundle.join("blobs")),
            ctx.fetch.limits(),
        )?);
        let loaded = load(&ctx.home, &identity, ctx.fetch.limits())?
            .ok_or_else(|| failure("import receipt vanished"))?;
        register(session, &identity, &loaded.source.prepared)?;
        return Ok(loaded.facts);
    }
    let stage = tempfile::Builder::new()
        .prefix("package-import-")
        .tempdir_in(&ctx.home)?;
    let retained = stage.path().join("receipt");
    std::fs::create_dir(&retained)?;
    std::fs::create_dir(retained.join("blobs"))?;
    let source_blobs = gripsack_fs::open_dir_nofollow(&source, Path::new("blobs"))?;
    for descriptor in closure.blobs.values() {
        receipt::blob(
            &source_blobs,
            descriptor,
            Some(&retained.join("blobs").join(&descriptor.sha256)),
        )?;
    }
    let conda_inputs =
        closure.preflight_conda(&ctx.home, Some(&retained.join("blobs")), ctx.fetch.limits())?;
    closure.write_records(&retained)?;
    let (payload, tree, conda, environment) =
        if matches!(manifest.layout, RuntimeLayout::Environment {}) {
            let (artifact, record) = super::environment::import(
                ctx,
                session,
                &closure,
                &retained.join("blobs"),
                &conda_inputs,
            )?;
            (artifact.payload, artifact.tree, None, Some(record))
        } else if matches!(manifest.layout, RuntimeLayout::Conda) {
            let component = &manifest.components[0];
            let record = component
                .conda
                .as_ref()
                .ok_or_else(|| failure("Conda layout missing archive set"))?;
            let value = super::conda::import(
                ctx,
                session,
                &component.id,
                record,
                &retained.join("blobs"),
                &conda_inputs,
            )?;
            let tree = check_payload(
                manifest,
                &closure,
                &value.artifact.payload,
                ctx.fetch.limits(),
            )?;
            (value.artifact.payload, tree, Some(value.receipt), None)
        } else {
            let aggregate = stage.path().join("payload");
            materialize_tree(ctx, &closure, manifest, &retained.join("blobs"), &aggregate)?;
            let tree = check_payload(manifest, &closure, &aggregate, ctx.fetch.limits())?;
            let payload = gripsack_store::content_path(&ctx.home, "package-payload", tree.as_str());
            if let RuntimeLayout::Fixed { prefix, .. } = &manifest.layout
                && Path::new(prefix) != payload
            {
                return Err(failure(
                    "fixed-layout destination differs from the declared exact prefix",
                ));
            }
            authorize(&verified, manifest)?;
            if artifact::retained_directory(&ctx.home, &payload)?.is_some() {
                if check_payload(manifest, &closure, &payload, ctx.fetch.limits())? != tree {
                    return Err(failure("existing portable payload is corrupt"));
                }
            } else {
                crate::source::publish(ctx, "portable payload", &aggregate, &payload)?;
            }
            (payload, tree, None, None)
        };
    authorize(&verified, manifest)?;
    let record = Receipt {
        version: 1,
        manifest: identity.clone(),
        realized_tree: tree.to_string(),
        payload: payload
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| failure("payload path is not UTF-8"))?
            .into(),
        trust_revision: verified.revision().into(),
        key_id: verified.key_id().into(),
        conda,
        external,
        environment,
    };
    receipt::write_record(&retained, &record)?;
    crate::source::publish(ctx, "portable receipt", &retained, &destination)?;
    let loaded = load(&ctx.home, &identity, ctx.fetch.limits())?
        .ok_or_else(|| failure("published receipt is absent"))?;
    register(session, &identity, &loaded.source.prepared)?;
    Ok(loaded.facts)
}
fn register(
    session: &LifecycleSession,
    identity: &str,
    source: &PreparedSource,
) -> Result<(), ExecError> {
    let retention = RetentionSet::admit(session, source.artifact.retention.iter().cloned())?;
    roots::register_output_root(
        session,
        &RootId::from_identity(format!("package-import:{identity}").as_bytes()),
        &retention,
    )
}

fn load(
    home: &Path,
    identity: &str,
    limits: FetchLimits,
) -> Result<Option<LoadedImport>, ExecError> {
    let root = imported_root(home, identity)?;
    let Some(directory) = artifact::retained_directory(home, &root)? else {
        return Ok(None);
    };
    let closure = Closure::read(&directory, limits)?;
    let (verified, external) = admit_closure(home, &closure)?;
    if closure.root != identity {
        return Err(failure(
            "retained manifest does not match requested identity",
        ));
    }
    let manifest = &closure.manifests[identity];
    let record = receipt::read_record(&directory)?;
    if record.version != 1 || record.manifest != identity {
        return Err(failure("materialization receipt identity mismatch"));
    }
    if record.external.keys().ne(external.keys()) {
        return Err(failure(
            "receipt independent-manifest inventory differs from closure",
        ));
    }
    let blobs = &closure.blobs;
    let blob_directory = gripsack_fs::open_dir_nofollow(&directory, Path::new("blobs"))?;
    for blob in blobs.values() {
        receipt::blob(&blob_directory, blob, None)?;
    }
    let conda_inputs = closure.preflight_conda(home, None, limits)?;
    let mut retention = BTreeSet::from([root.clone()]);
    let mut admitted_environment = None;
    let (payload, conda) = if matches!(manifest.layout, RuntimeLayout::Environment {}) {
        if record.conda.is_some() {
            return Err(failure(
                "environment root has unexpected single Conda receipt",
            ));
        }
        let evidence = record
            .environment
            .as_ref()
            .ok_or_else(|| failure("environment receipt missing"))?;
        let admitted = super::environment::inspect(
            home,
            &closure,
            evidence,
            &record.realized_tree,
            limits,
            &root,
            &conda_inputs,
        )?;
        retention.extend(admitted.artifact.retention.iter().cloned());
        let payload = admitted.artifact.payload.clone();
        admitted_environment = Some(admitted);
        (payload, None)
    } else if matches!(manifest.layout, RuntimeLayout::Conda) {
        let component = &manifest.components[0];
        let portable = component
            .conda
            .as_ref()
            .ok_or_else(|| failure("Conda archive record missing"))?;
        let value = super::conda::inspect_retained(&component.id, home, portable, &conda_inputs)?;
        if record.conda.as_ref() != Some(&value.receipt) {
            return Err(failure(
                "retained Conda realization differs from import receipt",
            ));
        }
        retention.extend(value.artifact.retention);
        (value.artifact.payload, Some(value.receipt))
    } else {
        if record.conda.is_some() {
            return Err(failure("tree import has unexpected Conda materialization"));
        }
        format::digest(&record.realized_tree)?;
        (
            gripsack_store::content_path(home, "package-payload", &record.realized_tree),
            None,
        )
    };
    if !matches!(manifest.layout, RuntimeLayout::Environment {}) && record.environment.is_some() {
        return Err(failure("package has unexpected environment receipt"));
    }
    if payload.file_name().and_then(|name| name.to_str()) != Some(record.payload.as_str()) {
        return Err(failure("materialization destination differs from receipt"));
    }
    if let RuntimeLayout::Fixed { prefix, .. } = &manifest.layout
        && Path::new(prefix) != payload
    {
        return Err(failure(
            "fixed-layout prefix differs from recipient destination",
        ));
    }
    if artifact::retained_directory(home, &payload)?.is_none() {
        return Err(failure("retained portable payload missing"));
    }
    let tree = if let Some(admitted) = &admitted_environment {
        admitted.artifact.tree.clone()
    } else {
        check_payload(manifest, &closure, &payload, limits)?
    };
    if tree.as_str() != record.realized_tree {
        return Err(failure("realized tree differs from import receipt"));
    }
    authorize(&verified, manifest)?;
    retention.insert(payload.clone());
    let facts = PackageFacts {
        manifest: identity.into(),
        publisher: manifest.publisher.clone(),
        namespace: manifest.namespace.clone(),
        trust_revision: verified.revision().into(),
        key_id: verified.key_id().into(),
        layout: manifest.layout.name().into(),
        target: manifest.target.clone(),
        external: external
            .into_iter()
            .map(|(identity, decision)| {
                let child = &closure.manifests[&identity];
                ExternalPackageFacts {
                    manifest: identity,
                    namespace: child.namespace.clone(),
                    publisher: child.publisher.clone(),
                    key_id: decision.key_id,
                    trust_revision: decision.trust_revision,
                    target: child.target.clone(),
                    layout: child.layout.name().into(),
                }
            })
            .collect(),
        realized_tree: tree.to_string(),
        materialization: conda.as_ref().map(|value| value.materialization.clone()),
        payload_digests: blobs.keys().map(|digest| (*digest).to_owned()).collect(),
        payload: payload.clone(),
    };
    let resolved = ResolvedPinFields {
        sha256: Some(identity.into()),
        tree256: Some(tree.to_string()),
        ..Default::default()
    };
    Ok(Some(LoadedImport {
        facts,
        source: AdmittedSource {
            prepared: PreparedSource {
                artifact: Arc::new(Artifact {
                    root: payload.clone(),
                    payload,
                    tree,
                    retention,
                }),
                resolved,
                conda,
            },
            environment: admitted_environment,
        },
    }))
}

pub fn inspect(home: &Path, manifest: &str) -> Result<Option<PackageFacts>, ExecError> {
    Ok(load(home, manifest, FetchLimits::default())?.map(|loaded| loaded.facts))
}
pub(super) fn admit_environment(
    home: &Path,
    manifest: &str,
    limits: FetchLimits,
) -> Result<Option<Arc<super::environment::AdmittedEnvironment>>, ExecError> {
    Ok(load(home, manifest, limits)?.and_then(|loaded| loaded.source.environment))
}
pub(super) fn admit_source(
    home: &Path,
    manifest: &str,
    limits: FetchLimits,
) -> Result<Option<AdmittedSource>, ExecError> {
    Ok(load(home, manifest, limits)?.map(|loaded| loaded.source))
}
pub(in crate::workspace) fn acquire(
    home: &Path,
    manifest: &str,
    limits: FetchLimits,
) -> Result<PreparedSource, ExecError> {
    load(home, manifest, limits)?
        .map(|loaded| loaded.source.prepared)
        .ok_or_else(|| failure("package is not imported; import its signed bundle first"))
}
pub fn readmit_roots<'a>(
    home: &Path,
    paths: impl IntoIterator<Item = &'a Path>,
) -> Result<(), ExecError> {
    for path in paths {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with("-package-import") {
            continue;
        }
        let directory = artifact::retained_directory(home, path)?
            .ok_or_else(|| failure("retained import receipt missing"))?;
        let bytes = receipt::read(
            &directory,
            Path::new("manifest.json"),
            format::MAX_MANIFEST_BYTES as u64,
        )?;
        let manifest = PortableManifest::decode(&bytes)?;
        let identity = manifest.digest()?;
        if imported_root(home, &identity)? != path {
            return Err(failure("import root is not bound to its manifest"));
        }
        acquire(home, &identity, FetchLimits::default())?;
    }
    Ok(())
}
/// An authored fixed prefix alone grants no placement authority. Only a
/// current-policy-admitted import at exactly its signed destination qualifies.
pub(in crate::workspace) fn readmit_fixed<'a>(
    home: &Path,
    payload: &Path,
    prefix: &Path,
    paths: impl IntoIterator<Item = &'a Path>,
) -> Result<(), ExecError> {
    if payload != prefix {
        return Err(failure(
            "fixed package prefix differs from its actual payload",
        ));
    }
    for path in paths {
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("-package-import"))
        {
            continue;
        }
        let directory = artifact::retained_directory(home, path)?
            .ok_or_else(|| failure("fixed package import receipt missing"))?;
        let bytes = receipt::read(
            &directory,
            Path::new("manifest.json"),
            format::MAX_MANIFEST_BYTES as u64,
        )?;
        let manifest = PortableManifest::decode(&bytes)?;
        let identity = manifest.digest()?;
        if imported_root(home, &identity)? != path {
            return Err(failure("fixed import root identity mismatch"));
        }
        let loaded = load(home, &identity, FetchLimits::default())?
            .ok_or_else(|| failure("fixed package import missing"))?;
        if (loaded.facts.layout == "fixed" && loaded.facts.payload == payload)
            || loaded
                .source
                .environment
                .as_ref()
                .is_some_and(|environment| environment.admits_fixed(payload))
        {
            return Ok(());
        }
    }
    Err(failure(
        "fixed prefix has no matching signed imported placement",
    ))
}

pub fn export_bundle(
    home: &Path,
    manifest: &str,
    output: &Path,
) -> Result<PackageFacts, ExecError> {
    let session = LifecycleSession::acquire(home)?;
    export_in_session(&session, manifest, output)
}

pub(super) fn export_in_session(
    session: &LifecycleSession,
    manifest: &str,
    output: &Path,
) -> Result<PackageFacts, ExecError> {
    let home = session.home();
    let loaded = load(home, manifest, FetchLimits::default())?
        .ok_or_else(|| failure("package is not imported"))?;
    let _retention = RetentionSet::admit(
        session,
        loaded.source.prepared.artifact.retention.iter().cloned(),
    )?;
    let root = imported_root(home, manifest)?;
    let directory = artifact::retained_directory(home, &root)?
        .ok_or_else(|| failure("import receipt missing"))?;
    let closure = Closure::read(&directory, FetchLimits::default())?;
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".package-export-")
        .tempdir_in(parent)?;
    std::fs::create_dir(stage.path().join("blobs"))?;
    let blobs = gripsack_fs::open_dir_nofollow(&directory, Path::new("blobs"))?;
    for descriptor in closure.blobs.values() {
        receipt::blob(
            &blobs,
            descriptor,
            Some(&stage.path().join("blobs").join(&descriptor.sha256)),
        )?;
    }
    closure.write_records(stage.path())?;
    gripsack_fs::publish_dir_at(stage.path(), output)?;
    Ok(loaded.facts)
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
