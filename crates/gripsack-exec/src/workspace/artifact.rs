//! Immutable native package objects and read-only historical recipe receipts.
//! Every publication requires lifecycle authority.
use crate::{Ctx, ExecError, LifecycleSession};
mod archive_root;
pub(super) mod hook;
pub(super) mod integrity;
#[cfg(test)]
#[path = "artifact/runtime_tests.rs"]
mod runtime_tests;
use gripsack_ir::{
    workspace::{RecipeOutputKind, WorkspacePlatform},
    workspace_model::{
        ArchiveRootPolicy, CatalogPackageLayout, HostRuntimeRequirements, PackageOutput,
        RecipeExecution,
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
const PACKAGE_RECEIPT_BASE: u32 = 1;
const PACKAGE_RECEIPT_HOST_RUNTIME: u32 = 2;
const PACKAGE_RECEIPT_ARCHIVE_ROOT: u32 = 3;
const PACKAGE_RECEIPT_PORTABLE: u32 = 4;
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
    pub archive_root: Option<ArchiveRootPolicy>,
    pub selected_root: Option<String>,
    pub selected_payload: Option<PathBuf>,
    pub commands: BTreeMap<String, ProvidedCommand>,
    pub runtime: Vec<Arc<Package>>,
    pub target: WorkspacePlatform,
    pub layout: CatalogPackageLayout,
    pub host_runtime: Option<HostRuntimeRequirements>,
    /// Conda runtime receipt for prefix-materialized packages (A3).
    pub conda: Option<super::conda::CondaRuntimeReceipt>,
}
impl Package {
    pub fn payload(&self) -> &Path {
        self.selected_payload
            .as_deref()
            .unwrap_or(&self.producer.payload)
    }
    pub fn host_dependent(&self) -> bool {
        self.any_in_runtime_closure(|package| package.host_runtime.is_some())
    }
    pub fn portable(&self) -> bool {
        self.any_in_runtime_closure(|package| {
            package.producer.retention.iter().any(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with("-package-import"))
            })
        })
    }

    /// Shared descendants must be queried once per immutable in-memory view,
    /// not recursively expanded along every incoming edge. A digest may be
    /// shared by distinct policy/retention views, so visitation uses borrowed
    /// object addresses only for this walk; no pointer enters persisted state.
    /// Local and leaf answers allocate nothing.
    fn any_in_runtime_closure(&self, mut predicate: impl FnMut(&Package) -> bool) -> bool {
        if predicate(self) {
            return true;
        }
        if self.runtime.is_empty() {
            return false;
        }
        let mut visited = BTreeSet::from([std::ptr::from_ref(self)]);
        let mut pending: Vec<_> = self.runtime.iter().map(Arc::as_ref).collect();
        while let Some(package) = pending.pop() {
            if !visited.insert(std::ptr::from_ref(package)) {
                continue;
            }
            if predicate(package) {
                return true;
            }
            pending.extend(package.runtime.iter().map(Arc::as_ref));
        }
        false
    }
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
            .map(|command| self.payload().join(&command.selector))
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
    layout: CatalogPackageLayout,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    conda: Option<super::conda::CondaRuntimeReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host_runtime: Option<HostRuntimeRequirements>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    runtime_receipts: BTreeMap<PackageDigest, gripsack_process::Sha256Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archive_root: Option<ArchiveRootPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selected_root: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    producer_retention: BTreeSet<PathBuf>,
}

impl PackageReceipt {
    // Keep policy-free v1 receipts byte-for-byte serializable for retained hooks.
    fn supported_version(&self) -> bool {
        if !self.producer_retention.is_empty() {
            return self.version == PACKAGE_RECEIPT_PORTABLE
                && self.archive_root.is_some() == self.selected_root.is_some();
        }
        self.version
            == if self.archive_root.is_some() && self.selected_root.is_some() {
                PACKAGE_RECEIPT_ARCHIVE_ROOT
            } else if self.archive_root.is_some() || self.selected_root.is_some() {
                return false;
            } else if self.host_runtime.is_some() || !self.runtime_receipts.is_empty() {
                PACKAGE_RECEIPT_HOST_RUNTIME
            } else {
                PACKAGE_RECEIPT_BASE
            }
    }
}

pub(super) struct ProducerIdentity<'a> {
    pub pin: &'a ResolvedPinFields,
    pub conda: Option<&'a super::conda::CondaRuntimeReceipt>,
}

/// One selected runtime closure in its realized and identity-binding forms.
pub(super) struct PackageRuntime<'a> {
    pub packages: Vec<Arc<Package>>,
    pub identities: &'a BTreeMap<String, PackageDigest>,
}
pub(super) fn publish_package(
    ctx: &Ctx,
    session: &LifecycleSession,
    declaration: &PackageOutput,
    producer: Arc<Artifact>,
    identity: ProducerIdentity<'_>,
    runtime: Vec<Arc<Package>>,
    runtime_names: &BTreeMap<String, PackageDigest>,
) -> Result<Arc<Package>, ExecError> {
    publish_package_with_context(
        ctx,
        session,
        declaration,
        producer,
        identity,
        PackageRuntime {
            packages: runtime,
            identities: runtime_names,
        },
        None,
    )
}

/// Internal realizations may retain authority-dependent evidence absent from
/// ordinary provider pins. Bind that evidence into their immutable identity;
/// declaration names alone are deliberately not semantic package identity.
/// Normal workspace callers retain their existing context-free digest.
pub(super) fn publish_package_with_context(
    ctx: &Ctx,
    session: &LifecycleSession,
    declaration: &PackageOutput,
    producer: Arc<Artifact>,
    identity: ProducerIdentity<'_>,
    runtime: PackageRuntime<'_>,
    context: Option<&[u8]>,
) -> Result<Arc<Package>, ExecError> {
    authority(ctx, session)?;
    let ProducerIdentity { pin, conda } = identity;
    let PackageRuntime {
        packages: runtime,
        identities: runtime_names,
    } = runtime;
    let identity = identity::provider_package_digest(declaration, pin, runtime_names)
        .map_err(|error| failure(error.to_string()))?;
    let identity = if let Some(context) = context {
        let mut digest = Sha256::new();
        digest.update(b"gripsack/portable-context-package/v1\0");
        digest.update(identity.bytes());
        digest.update(context);
        identity::PackageDigest::from_bytes(digest.finalize().into())
    } else {
        identity
    };
    let selected_root = archive_root::select(
        declaration.archive_root.as_ref(),
        &declaration.layout,
        &producer.payload,
    )?;
    if declaration.archive_root.is_some() && conda.is_some() {
        return Err(failure(
            "archive root selection cannot reinterpret a materialized prefix",
        ));
    }
    let commands = inspect_selected_commands(
        declaration,
        &archive_root::payload(&producer.payload, selected_root.as_deref()),
    )?;
    let root = store::content_path(&ctx.home, "workspace-package", &identity.to_string());
    let runtime_receipts = runtime_receipts(&ctx.home, declaration, &runtime)?;
    let producer_retention = portable_retention(&producer);
    let receipt = PackageReceipt {
        version: if !producer_retention.is_empty() {
            PACKAGE_RECEIPT_PORTABLE
        } else if declaration.archive_root.is_some() {
            PACKAGE_RECEIPT_ARCHIVE_ROOT
        } else if declaration.host_runtime.is_some() || !runtime_receipts.is_empty() {
            PACKAGE_RECEIPT_HOST_RUNTIME
        } else {
            PACKAGE_RECEIPT_BASE
        },
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
        conda: conda.cloned(),
        host_runtime: declaration.host_runtime.clone(),
        runtime_receipts,
        archive_root: declaration.archive_root,
        selected_root,
        producer_retention,
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
        selected_payload: receipt
            .selected_root
            .as_deref()
            .map(|name| producer.payload.join(name)),
        producer,
        archive_root: receipt.archive_root,
        selected_root: receipt.selected_root,
        commands: receipt.commands,
        runtime,
        target: receipt.target,
        layout: receipt.layout,
        conda: receipt.conda,
        host_runtime: receipt.host_runtime,
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
    let Some(receipt): Option<PackageReceipt> =
        read_receipt_readonly(home, &root, PACKAGE_RECEIPT)?
    else {
        return Ok(None);
    };
    let selected_root = archive_root::select(
        declaration.archive_root.as_ref(),
        &declaration.layout,
        &producer.payload,
    )?;
    let commands = inspect_selected_commands(
        declaration,
        &archive_root::payload(&producer.payload, selected_root.as_deref()),
    )?;
    let expected_runtime = runtime
        .iter()
        .map(|package| (package.identity, package.root.clone()))
        .collect();
    let runtime_receipts = runtime_receipts(home, declaration, &runtime)?;
    if !receipt.supported_version()
        || receipt.package != identity
        || receipt.producer_root != producer.root
        || receipt.producer_payload != producer.payload
        || receipt.tree != producer.tree
        || receipt.commands != commands
        || receipt.runtime != expected_runtime
        || receipt.target != declaration.target
        || receipt.layout != declaration.layout
        || receipt.conda != conda
        || receipt.host_runtime != declaration.host_runtime
        || receipt.runtime_receipts != runtime_receipts
        || receipt.archive_root != declaration.archive_root
        || receipt.selected_root != selected_root
        || receipt.producer_retention != portable_retention(&producer)
        || (receipt.archive_root.is_some() && conda.is_some())
    {
        return Err(failure(
            "retained package differs from its frozen identity/producer/runtime receipt",
        ));
    }
    Ok(Some(Arc::new(Package {
        identity,
        root,
        selected_payload: receipt
            .selected_root
            .as_deref()
            .map(|name| producer.payload.join(name)),
        producer,
        archive_root: receipt.archive_root,
        selected_root: receipt.selected_root,
        commands,
        runtime,
        target: declaration.target.clone(),
        layout: declaration.layout.clone(),
        conda,
        host_runtime: declaration.host_runtime.clone(),
    })))
}

fn portable_retention(producer: &Artifact) -> BTreeSet<PathBuf> {
    if producer.retention.iter().any(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("-package-import"))
    }) {
        producer.retention.clone()
    } else {
        BTreeSet::new()
    }
}

fn runtime_receipts(
    home: &Path,
    declaration: &PackageOutput,
    runtime: &[Arc<Package>],
) -> Result<BTreeMap<PackageDigest, gripsack_process::Sha256Digest>, ExecError> {
    let mut receipts = BTreeMap::new();
    if declaration.host_runtime.is_some() || runtime.iter().any(|package| package.host_dependent())
    {
        for package in runtime {
            hook::capture(home, package, &mut receipts)?;
        }
    }
    Ok(receipts)
}

pub(super) fn inspect_commands(
    declaration: &PackageOutput,
    payload: &Path,
) -> Result<BTreeMap<String, ProvidedCommand>, ExecError> {
    let selected_root = archive_root::select(
        declaration.archive_root.as_ref(),
        &declaration.layout,
        payload,
    )?;
    inspect_selected_commands(
        declaration,
        &archive_root::payload(payload, selected_root.as_deref()),
    )
}

fn inspect_selected_commands(
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

pub(super) fn retained_directory(
    home: &Path,
    root: &Path,
) -> Result<Option<gripsack_fs::Dir>, ExecError> {
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
    let Some(directory) = retained_directory(home, root)? else {
        return Ok(None);
    };
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
pub(super) fn write_receipt(
    directory: &Path,
    name: &str,
    receipt: &impl Serialize,
) -> Result<(), ExecError> {
    let bytes = serde_json::to_vec(receipt)?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(failure("artifact receipt exceeds its byte bound"));
    }
    gripsack_fs::atomic_write(&gripsack_fs::open(directory)?, Path::new(name), &bytes)?;
    Ok(())
}
fn failure(detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        output: "workspace".into(),
        step: "artifact".into(),
        detail: detail.into(),
    }
}
