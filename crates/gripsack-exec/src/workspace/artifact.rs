//! Immutable producer/package objects in the existing store. Operational worker
//! identities never enter receipts or keys, and payloads never point into a
//! disposable builder cache. Every publication requires lifecycle authority.
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_ir::{
    workspace::{RecipeOutputKind, WorkspacePlatform},
    workspace_v6::{
        PackageLayoutV6, PackageOutput, RecipeExecution,
        identity::{self, ExecutableDigest, PackageDigest, RecipeDigest},
        lock::ResolvedPinFields,
    },
};
use gripsack_store as store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

const RECEIPT_VERSION: u32 = 1;
const MAX_RECEIPT_BYTES: u64 = 1024 * 1024;
const RECIPE_RECEIPT: &str = "recipe.json";
const PACKAGE_RECEIPT: &str = "package.json";

pub(super) struct Artifact {
    pub root: PathBuf,
    pub payload: PathBuf,
    pub tree: store::hash::PayloadHash,
    pub retention: BTreeSet<PathBuf>,
}
impl From<super::acquire::AcquiredSource> for Artifact {
    fn from(source: super::acquire::AcquiredSource) -> Self {
        Self {
            payload: source.path.clone(),
            retention: BTreeSet::from([source.path.clone()]),
            root: source.path,
            tree: source.tree,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProvidedCommand {
    pub selector: String,
    pub executable: ExecutableDigest,
}
pub(super) struct Package {
    pub identity: PackageDigest,
    pub root: PathBuf,
    pub producer: Arc<Artifact>,
    pub commands: BTreeMap<String, ProvidedCommand>,
    pub runtime: Vec<Arc<Package>>,
    pub target: WorkspacePlatform,
    pub layout: PackageLayoutV6,
    /// Conda runtime receipt for prefix-materialized packages (A3).
    pub conda: Option<super::conda::CondaRuntimeReceipt>,
}
impl Package {
    pub fn retain_into(&self, roots: &mut BTreeSet<PathBuf>) {
        let mut pending = vec![self];
        while let Some(package) = pending.pop() {
            if !roots.insert(package.root.clone()) {
                continue;
            }
            roots.extend(package.producer.retention.iter().cloned());
            pending.extend(package.runtime.iter().map(Arc::as_ref));
        }
    }
    pub fn command(&self, name: &str) -> Option<PathBuf> {
        self.commands
            .get(name)
            .map(|command| self.producer.payload.join(&command.selector))
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeReceipt {
    version: u32,
    recipe: RecipeDigest,
    tree: store::hash::PayloadHash,
    output_kind: RecipeOutputKind,
    execution: RecipeExecution,
    retention: BTreeSet<PathBuf>,
}
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageReceipt {
    version: u32,
    package: PackageDigest,
    producer_root: PathBuf,
    producer_payload: PathBuf,
    tree: store::hash::PayloadHash,
    commands: BTreeMap<String, ProvidedCommand>,
    runtime: BTreeMap<PackageDigest, PathBuf>,
    target: WorkspacePlatform,
    layout: PackageLayoutV6,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    conda: Option<super::conda::CondaRuntimeReceipt>,
}

pub(super) fn cached_recipe(
    ctx: &Ctx,
    session: &LifecycleSession,
    recipe: RecipeDigest,
    kind: RecipeOutputKind,
    execution: &RecipeExecution,
    inputs: &BTreeSet<PathBuf>,
) -> Result<Option<Arc<Artifact>>, ExecError> {
    authority(ctx, session)?;
    retained_recipe(&ctx.home, ctx.fetch.limits(), recipe, kind, execution, inputs)
}

pub(super) fn retained_recipe(
    home: &Path,
    limits: gripsack_fetch::FetchLimits,
    recipe: RecipeDigest,
    kind: RecipeOutputKind,
    execution: &RecipeExecution,
    inputs: &BTreeSet<PathBuf>,
) -> Result<Option<Arc<Artifact>>, ExecError> {
    let root = store::content_path(home, "workspace-recipe", &recipe.to_string());
    let Some(receipt): Option<RecipeReceipt> = read_receipt_readonly(home, &root, RECIPE_RECEIPT)? else {
        return Ok(None);
    };
    if receipt.version != RECEIPT_VERSION
        || receipt.recipe != recipe
        || receipt.output_kind != kind
        || &receipt.execution != execution
        || &receipt.retention != inputs
    {
        return Err(failure("retained recipe identity/kind/policy differs"));
    }
    let payload = root.join("payload");
    let checked =
        super::stage::validate_output_tree(&payload, kind, "workspace-recipe", limits)?;
    if checked.tree_hash() != &receipt.tree {
        return Err(failure("retained recipe payload differs from its receipt"));
    }
    for retained in &receipt.retention {
        store::paths::validate_store_root(home, retained)?;
    }
    let mut retention = receipt.retention;
    retention.insert(root.clone());
    Ok(Some(Arc::new(Artifact {
        root,
        payload,
        tree: receipt.tree,
        retention,
    })))
}

pub(super) fn publish_recipe(
    ctx: &Ctx,
    session: &LifecycleSession,
    recipe: RecipeDigest,
    kind: RecipeOutputKind,
    execution: &RecipeExecution,
    stage: super::stage::ValidatedTree,
    inputs: BTreeSet<PathBuf>,
) -> Result<Arc<Artifact>, ExecError> {
    authority(ctx, session)?;
    for path in &inputs {
        store::paths::validate_store_root(&ctx.home, path)?;
    }
    if let Some(retained) = cached_recipe(ctx, session, recipe, kind, execution, &inputs)? {
        if retained.tree != *stage.tree_hash() {
            return Err(failure(
                "same recipe produced divergent bytes; retained output was not overwritten",
            ));
        }
        return Ok(retained);
    }
    let root = store::content_path(&ctx.home, "workspace-recipe", &recipe.to_string());
    let temporary = tempfile::Builder::new()
        .prefix("grip-workspace-publication-")
        .tempdir_in(
            stage
                .root()
                .parent()
                .ok_or_else(|| failure("validated staging has no parent"))?,
        )?;
    let object = temporary.path().join("object");
    std::fs::create_dir(&object)?;
    let receipt = RecipeReceipt {
        version: RECEIPT_VERSION,
        recipe,
        tree: stage.tree_hash().clone(),
        output_kind: kind,
        execution: execution.clone(),
        retention: inputs,
    };
    std::fs::rename(stage.root(), object.join("payload"))?;
    write_receipt(&object, RECIPE_RECEIPT, &receipt)?;
    crate::source::publish(ctx, "workspace-recipe", &object, &root)?;
    let mut retention = receipt.retention;
    retention.insert(root.clone());
    Ok(Arc::new(Artifact {
        payload: root.join("payload"),
        root,
        tree: receipt.tree,
        retention,
    }))
}

pub(super) enum ProducerIdentity<'a> {
    Recipe(RecipeDigest),
    Provider(&'a ResolvedPinFields),
}
pub(super) fn publish_package(
    ctx: &Ctx,
    session: &LifecycleSession,
    declaration: &PackageOutput,
    producer: Arc<Artifact>,
    identity: ProducerIdentity<'_>,
    runtime: Vec<Arc<Package>>,
    runtime_names: &BTreeMap<String, PackageDigest>,
    conda: Option<super::conda::CondaRuntimeReceipt>,
) -> Result<Arc<Package>, ExecError> {
    authority(ctx, session)?;
    let identity = match identity {
        ProducerIdentity::Recipe(recipe) => {
            identity::recipe_package_digest(declaration, recipe, runtime_names)
        }
        ProducerIdentity::Provider(pin) => {
            identity::provider_package_digest(declaration, pin, runtime_names)
        }
    }
    .map_err(|error| failure(error.to_string()))?;
    let commands = inspect_commands(declaration, &producer.payload)?;
    let root = store::content_path(&ctx.home, "workspace-package", &identity.to_string());
    let receipt = PackageReceipt {
        version: RECEIPT_VERSION,
        package: identity,
        producer_root: producer.root.clone(),
        producer_payload: producer.payload.clone(),
        tree: producer.tree.clone(),
        commands,
        runtime: runtime
            .iter()
            .map(|package| (package.identity, package.root.clone()))
            .collect(),
        target: declaration.target.clone(),
        layout: declaration.layout.clone(),
        conda,
    };
    match read_receipt::<PackageReceipt>(ctx, &root, PACKAGE_RECEIPT)? {
        Some(existing) if existing != receipt => {
            return Err(failure(
                "same package identity names a different producer/runtime/command receipt",
            ));
        }
        Some(_) => {}
        None => {
            let temporary = tempfile::Builder::new()
                .prefix("grip-workspace-package-")
                .tempdir()?;
            let object = temporary.path().join("object");
            std::fs::create_dir(&object)?;
            write_receipt(&object, PACKAGE_RECEIPT, &receipt)?;
            crate::source::publish(ctx, "workspace-package", &object, &root)?;
        }
    }
    Ok(Arc::new(Package {
        identity,
        root,
        producer,
        commands: receipt.commands,
        runtime,
        target: receipt.target,
        layout: receipt.layout,
        conda: receipt.conda,
    }))
}

/// Read matching package evidence without granting publication/runtime authority.
pub(super) fn retained_package(
    home: &Path,
    identity: PackageDigest,
    declaration: &PackageOutput,
    producer: Arc<Artifact>,
    runtime: Vec<Arc<Package>>,
    conda: Option<super::conda::CondaRuntimeReceipt>,
) -> Result<Option<Arc<Package>>, ExecError> {
    let root = store::content_path(home, "workspace-package", &identity.to_string());
    let Some(receipt): Option<PackageReceipt> = read_receipt_readonly(home, &root, PACKAGE_RECEIPT)? else {
        return Ok(None);
    };
    let commands = inspect_commands(declaration, &producer.payload)?;
    let expected_runtime = runtime.iter().map(|package| (package.identity, package.root.clone())).collect();
    if receipt.version != RECEIPT_VERSION
        || receipt.package != identity
        || receipt.producer_root != producer.root
        || receipt.producer_payload != producer.payload
        || receipt.tree != producer.tree
        || receipt.commands != commands
        || receipt.runtime != expected_runtime
        || receipt.target != declaration.target
        || receipt.layout != declaration.layout
        || receipt.conda != conda
    {
        return Err(failure("retained package differs from its frozen identity/producer/runtime receipt"));
    }
    Ok(Some(Arc::new(Package {
        identity, root, producer, commands, runtime,
        target: declaration.target.clone(), layout: declaration.layout.clone(), conda,
    })))
}

pub(super) fn inspect_commands(
    declaration: &PackageOutput,
    payload: &Path,
) -> Result<BTreeMap<String, ProvidedCommand>, ExecError> {
    let canonical_payload = payload.canonicalize()?;
    let directory = gripsack_fs::open(&canonical_payload)?;
    let mut commands = BTreeMap::new();
    for (name, selector) in &declaration.commands {
        let path = payload.join(selector).canonicalize()?;
        let relative = path
            .strip_prefix(&canonical_payload)
            .map_err(|_| failure("exported command escapes its admitted payload"))?;
        let mut file = gripsack_fs::open_file_nofollow(&directory, relative)?;
        let metadata = file.metadata()?;
        use gripsack_fs::cap_std::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(failure(format!(
                "exported command {name:?} is not executable"
            )));
        }
        let mut digest = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        commands.insert(
            name.clone(),
            ProvidedCommand {
                selector: selector.clone(),
                executable: ExecutableDigest::from_bytes(digest.finalize().into()),
            },
        );
    }
    Ok(commands)
}

pub(super) fn authority(ctx: &Ctx, session: &LifecycleSession) -> Result<(), ExecError> {
    if session.home() != ctx.home {
        return Err(failure(
            "publication belongs to another home's lifecycle authority",
        ));
    }
    Ok(())
}
pub(super) fn read_receipt<T: serde::de::DeserializeOwned>(
    ctx: &Ctx,
    root: &Path,
    name: &str,
) -> Result<Option<T>, ExecError> {
    read_receipt_readonly(&ctx.home, root, name)
}

pub(super) fn retained_directory(home: &Path, root: &Path) -> Result<Option<gripsack_fs::Dir>, ExecError> {
    store::paths::validate_store_root(home, root)?;
    let home = match gripsack_fs::open(home) {
        Ok(home) => home,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let parent = match gripsack_fs::open_dir_nofollow(&home, Path::new(store::STORE_DIR)) {
        Ok(parent) => parent,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let directory = match gripsack_fs::open_dir_nofollow(
        &parent,
        Path::new(
            root.file_name()
                .ok_or_else(|| failure("artifact root has no name"))?,
        ),
    ) {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(directory))
}

pub(super) fn read_receipt_readonly<T: serde::de::DeserializeOwned>(
    home: &Path,
    root: &Path,
    name: &str,
) -> Result<Option<T>, ExecError> {
    let Some(directory) = retained_directory(home, root)? else { return Ok(None); };
    let file = gripsack_fs::open_file_nofollow(&directory, Path::new(name))?;
    if file.metadata()?.len() > MAX_RECEIPT_BYTES {
        return Err(failure("artifact receipt exceeds its byte bound"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_RECEIPT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(failure("artifact receipt grew beyond its byte bound"));
    }
    Ok(Some(serde_json::from_slice(&bytes)?))
}
pub(super) fn write_receipt(directory: &Path, name: &str, receipt: &impl Serialize) -> Result<(), ExecError> {
    let bytes = serde_json::to_vec(receipt)?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(failure("artifact receipt exceeds its byte bound"));
    }
    gripsack_fs::atomic_write(&gripsack_fs::open(directory)?, Path::new(name), &bytes)?;
    Ok(())
}
fn failure(detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        module: "workspace".into(),
        step: "artifact".into(),
        detail: detail.into(),
    }
}
