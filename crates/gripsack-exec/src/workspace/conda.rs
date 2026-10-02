//! Coherent Conda acquisition, materialization and publication (A3).
//! Explicit update solves through the optional Rattler helper; the core
//! downloads every archive through the bounded FetchContext, verifies
//! SHA-256 and retains archives as flat store objects. Frozen
//! materialization NEVER solves: it re-derives the durable prefix from
//! the frozen lock + policy + canonical store context, stages through
//! the helper, then INDEPENDENTLY validates the tree and receipts from
//! the original archives before admission. Native and image consumers
//! share `materialize_locked` — image prefixes are an explicit separate
//! context, never a copy of native fixed-prefix bytes.

mod archive;
mod prefix;
mod runtime;
mod validate;

pub(super) use runtime::admit_native;
pub(crate) use validate::SystemRuntime;

use super::{
    artifact::Artifact,
    inputs,
    roots::{self, RetentionSet, RootId},
};
use crate::{Ctx, ExecError, LifecycleSession, UpdateMode};
use gripsack_conda::CondaHelper;
use gripsack_ir::{
    workspace::WorkspacePlatform,
    workspace_v6::{
        LockedSource, PixiLockSource, WorkspaceSourceV6, WorkspaceV6,
        identity::conda_closure_digest,
        lock::{
            LockedCondaEnvironment, LockedCondaPackage, LockedPin, LockedVirtualPackage,
            ResolvedPinFields,
        },
    },
};
use gripsack_store as store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};

/// Where a frozen closure is materialized.
pub(crate) enum CondaDestination<'a> {
    /// Durable native prefix: the materialization key is derived from
    /// the frozen closure + policy + canonical store context (never
    /// from the prefix itself), then the final content path follows.
    Native,
    /// Explicit image context: bytes are patched for `prefix` (the
    /// declared ImageDestination path) and staged under the caller's
    /// image staging. No store publication.
    Image { prefix: &'a str, staging: &'a Path },
}

/// The runtime receipt persisted with the package publication: exact
/// derivation inputs, the COMPLETE frozen closure (every install-critical
/// record field) and measured ambient needs. Consumer admission
/// re-derives the prefix from `materialization`, binds
/// `final_prefix == producer.root`, and validates host capability against
/// the full depends/constrains MatchSpecs in `closure.packages`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CondaRuntimeReceipt {
    /// Conda subdir, e.g. "linux-64".
    pub platform: String,
    /// 64-hex materialization key (see `materialization_key`).
    pub materialization: String,
    pub final_prefix: PathBuf,
    /// The exact frozen closure — also what image re-materialization
    /// consumes, so a package carries its complete lock authority.
    pub closure: LockedCondaEnvironment,
    /// Measured ambient runtime the closure cannot satisfy itself.
    #[serde(default, skip_serializing_if = "SystemRuntime::is_empty")]
    pub system: SystemRuntime,
}

/// A validated materialization: artifact plus receipt authority.
pub(super) struct CondaMaterialization {
    pub artifact: Artifact,
    pub receipt: CondaRuntimeReceipt,
}

fn failure(output: &str, detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        module: output.into(),
        step: "conda".into(),
        detail: detail.into(),
    }
}

/// Reject an incoherent frozen selection before looking up archives or helpers.
pub(super) fn admit_frozen(
    output: &str,
    source: &WorkspaceSourceV6,
    locked: &LockedCondaEnvironment,
) -> Result<(), ExecError> {
    let result = match source {
        WorkspaceSourceV6::CondaEnvironment(declared) => {
            gripsack_conda::virtuals::validate_frozen_request(
                locked,
                &root_specs(&declared.packages),
                &declared.channels,
            )
        }
        _ => gripsack_conda::virtuals::validate_frozen_environment(locked, None),
    };
    result.map_err(|error| failure(output, error.to_string()))
}

fn root_specs(packages: &BTreeMap<String, String>) -> Vec<String> {
    packages
        .iter()
        .map(|(name, constraint)| {
            if constraint.trim().is_empty() {
                name.clone()
            } else {
                format!("{name} {constraint}")
            }
        })
        .collect()
}

/// The store namespace for retained original archives — a flat
/// immediate child like every other workspace object, never a nested
/// alternate namespace.
const ARCHIVE_NAMESPACE: &str = "conda-archive";
/// The store namespace for materialized prefixes.
const PREFIX_NAMESPACE: &str = "workspace-conda";
/// Canonical store context version: the flat content-path layout the
/// materialization key is grounded in. A layout change is a new key.
const STORE_CONTEXT: &str = "store-flat-v1";

/// sha256(domain | closure digest | context fields), 64 lowercase hex.
/// The final prefix never enters its own key (no circular hash).
fn materialization_key(locked: &LockedCondaEnvironment, context: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gripsack/v6/conda-materialization");
    hasher.update(conda_closure_digest(locked).to_string().as_bytes());
    for field in context {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    gripsack_process::Sha256Digest::from_bytes(hasher.finalize().into()).to_string()
}

fn native_location(
    output: &str,
    home: &Path,
    locked: &LockedCondaEnvironment,
) -> Result<(String, PathBuf), ExecError> {
    let home = home.canonicalize()?;
    let context = home
        .to_str()
        .ok_or_else(|| failure(output, "canonical Conda store context is not UTF-8"))?;
    let key = materialization_key(locked, &["native", STORE_CONTEXT, context]);
    let prefix = store::content_path(&home, PREFIX_NAMESPACE, &key);
    Ok((key, prefix))
}

fn retained_archives(
    output: &str,
    home: &Path,
    locked: &LockedCondaEnvironment,
) -> Result<BTreeMap<String, PathBuf>, ExecError> {
    gripsack_conda::virtuals::validate_frozen_environment(locked, None)
        .map_err(|error| failure(output, error.to_string()))?;
    locked
        .packages
        .iter()
        .map(|record| {
            let path = verify_archive(output, home, &record.sha256)?;
            Ok((record.sha256.clone(), path))
        })
        .collect()
}

fn admitted_materialization(
    locked: &LockedCondaEnvironment,
    key: String,
    prefix: PathBuf,
    archives: &BTreeMap<String, PathBuf>,
    validated: validate::ValidatedTree,
    retain_prefix: bool,
) -> CondaMaterialization {
    let mut retention: std::collections::BTreeSet<PathBuf> = archives
        .values()
        .map(|path| {
            path.parent()
                .expect("retained archive has an object directory")
                .to_path_buf()
        })
        .collect();
    if retain_prefix {
        retention.insert(prefix.clone());
    }
    CondaMaterialization {
        artifact: Artifact {
            root: prefix.clone(),
            payload: prefix.clone(),
            tree: validated.tree,
            retention,
        },
        receipt: CondaRuntimeReceipt {
            platform: locked.platform.clone(),
            materialization: key,
            final_prefix: prefix,
            closure: locked.clone(),
            system: validated.system,
        },
    }
}

/// Image derivation is independent of the native home, but binds the complete
/// frozen materialization policy and its declared final destination.
pub(super) fn image_key(locked: &LockedCondaEnvironment, prefix: &str) -> String {
    materialization_key(locked, &["image", prefix])
}

/// Reconstruct image materialization authority from original retained archives,
/// without invoking the helper or treating a cached receipt as proof.
pub(super) fn inspect_image(
    output: &str,
    home: &Path,
    locked: &LockedCondaEnvironment,
    prefix: &str,
    payload: &Path,
) -> Result<CondaMaterialization, ExecError> {
    let archives = retained_archives(output, home, locked)?;
    let validated = validate::validate_tree(
        output,
        locked,
        &archives,
        payload,
        prefix,
        validate::FileModes::Published,
    )?;
    let mut admitted = admitted_materialization(
        locked,
        image_key(locked, prefix),
        PathBuf::from(prefix),
        &archives,
        validated,
        false,
    );
    admitted.artifact.root = payload.to_path_buf();
    admitted.artifact.payload = payload.to_path_buf();
    Ok(admitted)
}

fn retained_prefix_exists(output: &str, prefix: &Path) -> Result<bool, ExecError> {
    match std::fs::symlink_metadata(prefix) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(failure(output, "retained Conda prefix is not a directory")),
        Err(error) => Err(error.into()),
    }
}

/// Read-only admission of an existing native prefix. Absence is deferred
/// production; present but corrupt/incomplete evidence is an error, never repair.
pub(super) fn inspect_retained(
    output: &str,
    home: &Path,
    locked: &LockedCondaEnvironment,
) -> Result<Option<CondaMaterialization>, ExecError> {
    let (key, prefix) = match native_location(output, home, locked) {
        Err(ExecError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        result => result?,
    };
    if !retained_prefix_exists(output, &prefix)? {
        return Ok(None);
    }
    let archives = retained_archives(output, home, locked)?;
    let prefix_text = prefix
        .to_str()
        .ok_or_else(|| failure(output, "materialization prefix is not UTF-8"))?;
    let validated = validate::validate_tree(
        output,
        locked,
        &archives,
        &prefix,
        prefix_text,
        validate::FileModes::Published,
    )?;
    Ok(Some(admitted_materialization(
        locked, key, prefix, &archives, validated, true,
    )))
}

fn archive_object(home: &Path, sha256: &str) -> PathBuf {
    store::content_path(home, ARCHIVE_NAMESPACE, sha256)
}

fn archive_file(home: &Path, sha256: &str) -> PathBuf {
    archive_object(home, sha256).join("archive")
}

/// Rehash one retained archive against its locked digest. Missing is a
/// missing-input error; a mismatch is corruption, never re-download.
fn verify_archive(output: &str, home: &Path, sha256: &str) -> Result<PathBuf, ExecError> {
    let path = archive_file(home, sha256);
    let file = std::fs::File::open(&path).map_err(|error| {
        failure(
            output,
            format!(
                "retained conda archive {sha256} is unavailable ({error}); restore it or re-run grip update"
            ),
        )
    })?;
    let mut hasher = Sha256::new();
    let mut file = file.take(u64::MAX);
    let mut buffer = [0u8; 65_536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = gripsack_process::Sha256Digest::from_bytes(hasher.finalize().into()).to_string();
    if actual != sha256 {
        return Err(failure(
            output,
            format!("retained conda archive {sha256} differs from its locked digest"),
        ));
    }
    Ok(path)
}

/// The optional helper: an explicit operator override first
/// (`GRIPSACK_CONDA_HELPER`, same authority as `--bridge`), else the
/// pinned lazy provisioning service. Absence of both is an explicit
/// error, never a silent fallback to ambient binaries.
fn helper(ctx: &Ctx, output: &str) -> Result<CondaHelper, ExecError> {
    let deadline = std::time::Instant::now()
        .checked_add(gripsack_process::Limits::default().timeout)
        .ok_or_else(|| failure(output, "Conda helper deadline overflow"))?;
    let (path, expected) = match std::env::var_os("GRIPSACK_CONDA_HELPER") {
        Some(path) if !path.is_empty() => (PathBuf::from(path), None),
        _ => {
            let provisioned = gripsack_conda::provision::ensure(&ctx.home, &ctx.fetch)
                .map_err(|error| failure(output, error.to_string()))?;
            let pin = gripsack_process::Sha256Digest::parse(&provisioned.pin)?;
            (provisioned.path, Some(pin))
        }
    };
    CondaHelper::select(&path, expected, ctx.repository.contents(), deadline)
        .map_err(|error| failure(output, format!("Conda helper selection: {error}")))
}

/// The conda subdir for a target platform. Conda linux binaries are
/// glibc-linked; a musl target is an explicit refusal, never a guess.
fn subdir(output: &str, platform: &WorkspacePlatform) -> Result<&'static str, ExecError> {
    use gripsack_ir::workspace::{PlatformAbi, PlatformArch, PlatformOs};
    match (platform.os, platform.arch) {
        (PlatformOs::Linux, PlatformArch::X86_64) => match platform.abi {
            Some(PlatformAbi::Musl) => Err(failure(
                output,
                "conda linux-64 binaries require a glibc target, not musl",
            )),
            _ => Ok("linux-64"),
        },
        (PlatformOs::Linux, PlatformArch::Aarch64) => match platform.abi {
            Some(PlatformAbi::Musl) => Err(failure(
                output,
                "conda linux-aarch64 binaries require a glibc target, not musl",
            )),
            _ => Ok("linux-aarch64"),
        },
        (PlatformOs::Macos, PlatformArch::X86_64) => Ok("osx-64"),
        (PlatformOs::Macos, PlatformArch::Aarch64) => Ok("osx-arm64"),
    }
}

/// Measured host facts a solve is grounded in. A foreign target needs its
/// own explicit runtime facts; local measurements are never substituted.
fn host_virtual_packages(
    output: &str,
    platform: &WorkspacePlatform,
) -> Result<Vec<LockedVirtualPackage>, ExecError> {
    use gripsack_ir::workspace::{PlatformArch, PlatformOs};
    let host_os = match std::env::consts::OS {
        "linux" => PlatformOs::Linux,
        "macos" => PlatformOs::Macos,
        other => {
            return Err(failure(output, format!("unsupported conda host {other}")));
        }
    };
    let host_arch = match std::env::consts::ARCH {
        "x86_64" => PlatformArch::X86_64,
        "aarch64" => PlatformArch::Aarch64,
        other => {
            return Err(failure(
                output,
                format!("unsupported conda host arch {other}"),
            ));
        }
    };
    if platform.os != host_os || platform.arch != host_arch {
        return Err(failure(
            output,
            "cross-platform conda solving requires explicit target runtime facts, not measurements from this host",
        ));
    }
    let fact = |name: &str, version: String, build: &str| LockedVirtualPackage {
        name: name.into(),
        version,
        build: build.into(),
    };
    let mut facts = vec![fact("__unix", "0".into(), "0")];
    let release = crate::facts::platform_release()
        .map_err(|error| failure(output, format!("measuring native OS release: {error}")))?;
    match platform.os {
        PlatformOs::Linux => {
            facts.push(fact("__linux", release.version.clone(), "0"));
            if let Some(glibc) = crate::facts::detect()
                .libc
                .as_deref()
                .and_then(|value| value.strip_prefix("glibc-"))
            {
                facts.push(fact("__glibc", glibc.into(), "0"));
            }
        }
        PlatformOs::Macos => facts.push(fact("__osx", release.version.clone(), "0")),
    }
    // Solving uses a portable architecture baseline. Native admission measures
    // the physical CPU separately; a solve's assumptions are not capabilities.
    facts.push(fact(
        "__archspec",
        "1".into(),
        match platform.arch {
            PlatformArch::X86_64 => "x86_64",
            PlatformArch::Aarch64 => "aarch64",
        },
    ));
    Ok(facts)
}

/// Retain one archive as a verified flat store object. Existing objects
/// are re-verified, never re-downloaded; new objects are downloaded
/// through the bounded FetchContext (sha256-verified) and published
/// through the common publication authority.
fn retain_archive(
    output: &str,
    ctx: &Ctx,
    record: &LockedCondaPackage,
) -> Result<PathBuf, ExecError> {
    let object = archive_object(&ctx.home, &record.sha256);
    if object.is_dir() {
        return verify_archive(output, &ctx.home, &record.sha256).map(|_| object);
    }
    let download = ctx
        .fetch
        .download_verified(&record.url, &record.sha256)
        .map_err(|error| failure(output, format!("{} archive download: {error}", record.name)))?;
    let stage = tempfile::Builder::new()
        .prefix("grip-conda-archive-")
        .tempdir()?;
    let payload = stage.path().join("archive");
    download
        .file
        .persist(&payload)
        .map_err(|error| failure(output, format!("archive spool: {}", error.error)))?;
    crate::source::publish(ctx, ARCHIVE_NAMESPACE, stage.path(), &object)?;
    Ok(object)
}

/// Materialize a frozen closure — the common native/image service.
/// Never solves. Archive presence and digests are verified first; the
/// staged tree is independently validated from the original archives
/// before it is admitted anywhere.
pub(super) fn materialize_locked(
    ctx: &Ctx,
    session: &LifecycleSession,
    output: &str,
    locked: &LockedCondaEnvironment,
    dest: CondaDestination<'_>,
) -> Result<CondaMaterialization, ExecError> {
    if session.home() != ctx.home {
        return Err(failure(
            output,
            "conda materialization belongs to another home",
        ));
    }
    let (key, final_prefix, image_staging) = match &dest {
        CondaDestination::Native => {
            let (key, prefix) = native_location(output, &ctx.home, locked)?;
            (key, prefix, None)
        }
        CondaDestination::Image { prefix, staging } => {
            let key = image_key(locked, prefix);
            (key, PathBuf::from(prefix), Some(*staging))
        }
    };
    let prefix_text = final_prefix
        .to_str()
        .ok_or_else(|| failure(output, "materialization prefix is not UTF-8"))?
        .to_owned();
    // 1. Verify every retained archive before any helper runs.
    let archives = retained_archives(output, &ctx.home, locked)?;
    // 2. Idempotent native reuse: an existing prefix is re-validated
    //    from the original archives, exactly like retained sources.
    if image_staging.is_none() && retained_prefix_exists(output, &final_prefix)? {
        let validated = validate::validate_tree(
            output,
            locked,
            &archives,
            &final_prefix,
            &prefix_text,
            validate::FileModes::Published,
        )?;
        return Ok(admitted_materialization(
            locked,
            key,
            final_prefix,
            &archives,
            validated,
            true,
        ));
    }
    // 3. Stage through the helper, then validate independently.
    let owned_staging;
    let staging = match image_staging {
        Some(staging) => {
            std::fs::create_dir_all(staging)?;
            staging.to_path_buf()
        }
        None => {
            owned_staging = tempfile::Builder::new()
                .prefix("grip-conda-materialize-")
                .tempdir()?;
            owned_staging.path().to_path_buf()
        }
    };
    let mut attempt = [0u8; 8];
    getrandom::fill(&mut attempt).map_err(std::io::Error::other)?;
    let archive_args: Vec<(String, PathBuf)> = archives
        .iter()
        .map(|(sha256, path)| (sha256.clone(), path.clone()))
        .collect();
    let helper = helper(ctx, output)?;
    helper
        .materialize(
            u64::from_le_bytes(attempt),
            locked,
            &final_prefix,
            &staging,
            &archive_args,
        )
        .map_err(|error| failure(output, format!("conda materialize: {error}")))?;
    let validated = validate::validate_tree(
        output,
        locked,
        &archives,
        &staging,
        &prefix_text,
        validate::FileModes::Staged,
    )?;
    // 4. Native publication through the common authority; image trees
    //    stay in the caller's staging.
    if image_staging.is_none() {
        crate::source::publish(ctx, PREFIX_NAMESPACE, &staging, &final_prefix)?;
    }
    let mut admitted = admitted_materialization(
        locked,
        key,
        final_prefix,
        &archives,
        validated,
        image_staging.is_none(),
    );
    if image_staging.is_some() {
        admitted.artifact.root = staging.clone();
        admitted.artifact.payload = staging;
    }
    Ok(admitted)
}

/// Explicit update: solve/import through the helper, retain every
/// archive as a verified store object, and return the lock v2 pin. The
/// caller upserts and publishes the lock; roots protect the archives
/// from that moment (no GC window before the first realization).
pub(crate) fn resolve_update(
    ctx: &Ctx,
    session: &LifecycleSession,
    name: &str,
    source: &WorkspaceSourceV6,
    platform: &WorkspacePlatform,
    workspace: &WorkspaceV6,
    mode: UpdateMode,
) -> Result<LockedPin, ExecError> {
    let subdir = subdir(name, platform)?;
    let mut attempt = [0u8; 8];
    getrandom::fill(&mut attempt).map_err(std::io::Error::other)?;
    let attempt = u64::from_le_bytes(attempt);
    let mut locked_source = source.locked();
    let environment = match source {
        WorkspaceSourceV6::CondaEnvironment(declared) => {
            if !declared.platforms.is_empty() && !declared.platforms.contains(platform) {
                return Err(failure(
                    name,
                    "requesting platform is not in the declared conda platforms",
                ));
            }
            let facts = host_virtual_packages(name, platform)?;
            let helper = helper(ctx, name)?;
            let environment = helper
                .resolve(
                    attempt,
                    &declared.channels,
                    &declared.packages,
                    subdir,
                    &facts,
                )
                .map_err(|error| failure(name, format!("conda solve: {error}")))?;
            gripsack_conda::virtuals::validate_solved_environment(
                &environment,
                &root_specs(&declared.packages),
                &declared.channels,
                &facts,
            )
            .map_err(|error| failure(name, error.to_string()))?;
            environment
        }
        WorkspaceSourceV6::PixiLock(declared) => {
            let read_input = |input_name: &str| -> Result<(Vec<u8>, String), ExecError> {
                let declaration = workspace
                    .inputs
                    .iter()
                    .find(|input| input.name == *input_name)
                    .ok_or_else(|| {
                        failure(
                            name,
                            format!("pixi import names unknown input {input_name:?}"),
                        )
                    })?;
                let captured = inputs::capture(ctx, session, declaration)?;
                let bytes = std::fs::read(captured.binding_path())?;
                Ok((bytes, captured.identity.to_string()))
            };
            let (manifest, manifest_sha256) = read_input(&declared.manifest)?;
            let (lock, lock_sha256) = read_input(&declared.lock)?;
            let manifest = String::from_utf8(manifest)
                .map_err(|_| failure(name, "pixi manifest is not UTF-8"))?;
            let lock =
                String::from_utf8(lock).map_err(|_| failure(name, "pixi lock is not UTF-8"))?;
            let helper = helper(ctx, name)?;
            let environment = helper
                .import_pixi(attempt, &manifest, &lock, &declared.environment, subdir)
                .map_err(|error| failure(name, format!("pixi import: {error}")))?;
            if let LockedSource::PixiLock(locked) = &mut locked_source {
                locked.manifest_sha256 = Some(manifest_sha256);
                locked.lock_sha256 = Some(lock_sha256);
            }
            environment
        }
        WorkspaceSourceV6::Fetch(_) => {
            return Err(failure(name, "fetch sources do not resolve through conda"));
        }
    };
    if environment.platform != subdir {
        return Err(failure(
            name,
            format!(
                "helper returned subdir {:?} for requested {subdir:?}",
                environment.platform
            ),
        ));
    }
    admit_frozen(name, source, &environment)?;
    // Retain every archive before the pin can name it. Check mode
    // verifies availability without publishing store objects or roots.
    let mut objects = Vec::with_capacity(environment.packages.len());
    for record in &environment.packages {
        if mode == UpdateMode::Publish {
            objects.push(retain_archive(name, ctx, record)?);
        } else {
            ctx.fetch
                .download_verified(&record.url, &record.sha256)
                .map_err(|error| {
                    failure(name, format!("{} archive download: {error}", record.name))
                })?;
        }
    }
    if mode == UpdateMode::Publish {
        let retention: std::collections::BTreeSet<PathBuf> = objects.into_iter().collect();
        let retention = RetentionSet::admit(session, retention)?;
        let id = RootId::from_identity(&serde_json::to_vec(&(ctx.repository.identity(), name))?);
        roots::register_output_root(session, &id, &retention)?;
    }
    let digest = conda_closure_digest(&environment).to_string();
    Ok(LockedPin {
        output: name.into(),
        source: locked_source,
        resolved: ResolvedPinFields {
            sha256: Some(digest),
            ..Default::default()
        },
        conda: Some(environment),
    })
}

/// Frozen pixi freshness: re-capture the declared inputs and compare
/// their content identities against the lock-recorded digests. A
/// changed manifest or lock document can never silently reuse a closure
/// solved against different bytes.
pub(crate) fn verify_pixi_inputs(
    ctx: &Ctx,
    session: &LifecycleSession,
    workspace: &WorkspaceV6,
    output: &str,
    declared: &PixiLockSource,
    pin: &LockedPin,
) -> Result<(), ExecError> {
    let LockedSource::PixiLock(locked) = &pin.source else {
        return Err(failure(output, "pixi pin lost its locked source"));
    };
    for (input_name, recorded) in [
        (&declared.manifest, &locked.manifest_sha256),
        (&declared.lock, &locked.lock_sha256),
    ] {
        let declaration = workspace
            .inputs
            .iter()
            .find(|input| input.name == *input_name)
            .ok_or_else(|| {
                failure(
                    output,
                    format!("pixi import names unknown input {input_name:?}"),
                )
            })?;
        let captured = inputs::capture(ctx, session, declaration)?;
        let expected = recorded.as_deref().ok_or_else(|| {
            failure(
                output,
                format!("pixi input {input_name:?} has no lock-recorded identity; run grip update"),
            )
        })?;
        if captured.identity.to_string() != expected {
            return Err(failure(
                output,
                format!("pixi input {input_name:?} changed after its lock; run grip update"),
            ));
        }
    }
    Ok(())
}
