//! Original-archive Conda sharing, never an installed-prefix transfer.
//!
//! The complete admitted lock and the core-pinned static helper are portable
//! inputs. Imports spool and verify every input before publishing anything,
//! then use the ordinary independently validated final-prefix materializer.
//! No helper override, provisioning, solve, repodata or pip path is reachable.
//! Binary Conda records and hash-pinned binary wheels retain their original
//! archives. Source distributions and editable installs are never admitted.
//! Core-approved redistribution notices accompany the helper and remain
//! retained, digest-verified original inputs at every managed readmission.
#[path = "conda_notices.rs"]
mod notices;

use super::format::{BlobDescriptor, MAX_ENTRIES, MAX_PAYLOAD_BYTES};
use crate::workspace::conda::{self, CondaMaterialization};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_ir::workspace::{PlatformAbi, PlatformArch, PlatformOs, WorkspacePlatform};
use gripsack_ir::workspace_model::lock::LockedCondaEnvironment;
use gripsack_process::Sha256Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Instant,
};

pub(super) const ARCHIVE_MEDIA_TYPE: &str = "application/vnd.gripsack.conda.archive.v1";
pub(super) const WHEEL_MEDIA_TYPE: &str = "application/vnd.gripsack.python.wheel.v1";
pub(super) const HELPER_MEDIA_TYPE: &str = "application/vnd.gripsack.conda.helper.v1";
const HELPER_NAMESPACE: &str = "portable-conda-helper";
const NATIVE_TARGET: WorkspacePlatform = WorkspacePlatform {
    os: PlatformOs::Linux,
    arch: PlatformArch::X86_64,
    abi: Some(PlatformAbi::Gnu),
    minimum_os: None,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableConda {
    pub lock: LockedCondaEnvironment,
    /// Canonical digest order, one descriptor per original Conda archive/wheel.
    pub archives: Vec<BlobDescriptor>,
    /// Raw static executable, authenticated against this core's release pin.
    pub helper: BlobDescriptor,
    /// Exact named notices for this core-approved helper, not publisher text.
    pub helper_notices: BTreeMap<String, BlobDescriptor>,
}

fn failure(output: &str, detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        output: output.into(),
        step: "portable conda".into(),
        detail: detail.into(),
    }
}

impl PortableConda {
    /// Every original input bound by this environment, including bootstrap
    /// notices. Consumers deduplicate identical bytes by descriptor digest.
    pub fn descriptors(&self) -> impl Iterator<Item = &BlobDescriptor> {
        self.archives
            .iter()
            .chain(std::iter::once(&self.helper))
            .chain(self.helper_notices.values())
    }

    pub(super) fn approved_helper_notices() -> &'static BTreeMap<String, BlobDescriptor> {
        notices::descriptors()
    }

    /// Pure lock/descriptor admission; the helper identity is core authority,
    /// not authority conferred by the publisher's signature.
    pub(super) fn validate(&self) -> Result<(), ExecError> {
        let output = "conda bundle";
        if &self.helper_notices != Self::approved_helper_notices() {
            return Err(failure(
                output,
                "bundled Conda helper notices differ from the core-approved inventory",
            ));
        }
        if self.archives.len() > MAX_ENTRIES
            || self
                .lock
                .packages
                .len()
                .checked_add(self.lock.wheels.len())
                .is_none_or(|count| count > MAX_ENTRIES)
        {
            return Err(failure(
                output,
                "Conda archive count exceeds portable bound",
            ));
        }
        let total = self
            .descriptors()
            .try_fold(0u64, |total, descriptor| total.checked_add(descriptor.size));
        if total.is_none_or(|total| total > MAX_PAYLOAD_BYTES) {
            return Err(failure(
                output,
                "Conda input bytes exceed portable payload bound",
            ));
        }
        gripsack_conda::virtuals::validate_frozen_environment(&self.lock, None)
            .map_err(|error| failure(output, error.to_string()))?;
        if self.lock.platform != "linux-64" {
            return Err(failure(output, "portable Conda requires linux-64"));
        }
        if self.helper.media_type != HELPER_MEDIA_TYPE
            || self.helper.sha256 != gripsack_conda::provision::approved_pin()?.to_string()
            || self.helper.size == 0
        {
            return Err(failure(
                output,
                "bundled Conda helper does not match the core-approved pin and executable contract",
            ));
        }
        let mut previous: Option<&str> = None;
        for descriptor in &self.archives {
            Sha256Digest::parse(&descriptor.sha256)?;
            if !matches!(
                descriptor.media_type.as_str(),
                ARCHIVE_MEDIA_TYPE | WHEEL_MEDIA_TYPE
            ) || descriptor.size == 0
                || previous.is_some_and(|digest| digest >= descriptor.sha256.as_str())
            {
                return Err(failure(
                    output,
                    "Conda archives must have canonical unique digest order, nonzero sizes and an admitted archive media type",
                ));
            }
            previous = Some(&descriptor.sha256);
        }
        let required: BTreeSet<_> = self
            .lock
            .packages
            .iter()
            .map(|record| record.sha256.as_str())
            .chain(self.lock.wheels.iter().map(|record| record.sha256.as_str()))
            .collect();
        if required.len() != self.archives.len()
            || !self
                .archives
                .iter()
                .all(|descriptor| required.contains(descriptor.sha256.as_str()))
        {
            return Err(failure(
                output,
                "Conda archive descriptors do not match the complete locked closure",
            ));
        }
        for record in &self.lock.packages {
            self.validate_descriptor(
                &record.name,
                &record.sha256,
                record.size,
                ARCHIVE_MEDIA_TYPE,
            )?;
        }
        for wheel in &self.lock.wheels {
            self.validate_descriptor(&wheel.name, &wheel.sha256, wheel.size, WHEEL_MEDIA_TYPE)?;
        }
        Ok(())
    }

    fn validate_descriptor(
        &self,
        name: &str,
        digest: &str,
        size: Option<u64>,
        media_type: &str,
    ) -> Result<(), ExecError> {
        let descriptor = self
            .archives
            .binary_search_by(|entry| entry.sha256.as_str().cmp(digest))
            .map(|index| &self.archives[index])
            .map_err(|_| failure("conda bundle", "missing locked Conda archive descriptor"))?;
        if descriptor.media_type != media_type {
            return Err(failure(
                "conda bundle",
                format!("archive {name} media type differs from its locked record kind"),
            ));
        }
        if size.is_some_and(|size| size != descriptor.size) {
            return Err(failure(
                "conda bundle",
                format!("Conda archive {name} size differs from its lock"),
            ));
        }
        Ok(())
    }
}

/// Export callers hold the lifecycle session/root protecting the source
/// receipt. Only helper bootstrap may acquire bytes; package archives must
/// already exist in the retained original archive inventory.
pub(super) fn export_inventory(
    ctx: &Ctx,
    output: &str,
    lock: &LockedCondaEnvironment,
) -> Result<(PortableConda, BTreeMap<String, PathBuf>), ExecError> {
    let mut paths = conda::retained_archives(output, &ctx.home, lock)?;
    let wheels: BTreeSet<_> = lock
        .wheels
        .iter()
        .map(|wheel| wheel.sha256.as_str())
        .collect();
    let archives = paths
        .iter()
        .map(|(digest, path)| {
            let media_type = if wheels.contains(digest.as_str()) {
                WHEEL_MEDIA_TYPE
            } else {
                ARCHIVE_MEDIA_TYPE
            };
            describe(output, path, digest, media_type)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let pin = gripsack_conda::provision::approved_pin()?.to_string();
    let retained_helper = helper_object(&ctx.home, &pin).join("gripsack-conda");
    let helper_path = match std::fs::symlink_metadata(&retained_helper) {
        Ok(_) => retained_helper,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            gripsack_conda::provision::ensure(&ctx.home, &ctx.fetch)?.path
        }
        Err(error) => return Err(error.into()),
    };
    let helper = describe(output, &helper_path, &pin, HELPER_MEDIA_TYPE)?;
    paths.insert(pin, helper_path);
    paths.extend(notices::export(ctx, output)?);
    let portable = PortableConda {
        lock: lock.clone(),
        archives,
        helper,
        helper_notices: PortableConda::approved_helper_notices().clone(),
    };
    portable.validate()?;
    Ok((portable, paths))
}

fn open_regular(output: &str, path: &Path) -> Result<File, ExecError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| {
            failure(
                output,
                format!(
                    "offline Conda input {} is unavailable: {error}",
                    path.display()
                ),
            )
        })?;
    if !file.metadata()?.is_file() {
        return Err(failure(output, "offline Conda input is not a regular file"));
    }
    Ok(file)
}

fn describe(
    output: &str,
    path: &Path,
    digest: &str,
    media_type: &str,
) -> Result<BlobDescriptor, ExecError> {
    let metadata = open_regular(output, path)?.metadata()?;
    if media_type == HELPER_MEDIA_TYPE
        && !matches!(metadata.permissions().mode() & 0o7777, 0o755 | 0o555)
    {
        return Err(failure(
            output,
            "retained Conda helper does not have the core-approved executable mode",
        ));
    }
    let descriptor = BlobDescriptor {
        sha256: digest.into(),
        size: metadata.len(),
        media_type: media_type.into(),
    };
    verify_copy(output, path, &descriptor, &mut std::io::sink())?;
    Ok(descriptor)
}

/// Bound every read to the declared length plus one, hash the exact private
/// spool bytes, and reject aliases/special files before reading them.
fn verify_copy(
    output: &str,
    source: &Path,
    descriptor: &BlobDescriptor,
    target: &mut impl Write,
) -> Result<(), ExecError> {
    if descriptor.size > MAX_PAYLOAD_BYTES {
        return Err(failure(
            output,
            "offline Conda blob exceeds portable payload bound",
        ));
    }
    let expected = Sha256Digest::parse(&descriptor.sha256)?;
    let file = open_regular(output, source)?;
    if file.metadata()?.len() != descriptor.size {
        return Err(failure(
            output,
            format!("offline Conda blob {} size mismatch", descriptor.sha256),
        ));
    }
    let bound = descriptor
        .size
        .checked_add(1)
        .ok_or_else(|| failure(output, "offline Conda blob size overflow"))?;
    let mut file = file.take(bound);
    let mut hasher = Sha256::new();
    let mut copied = 0u64;
    let mut buffer = [0u8; 65_536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        copied += count as u64;
        hasher.update(&buffer[..count]);
        target.write_all(&buffer[..count])?;
    }
    if copied != descriptor.size || Sha256Digest::from_bytes(hasher.finalize().into()) != expected {
        return Err(failure(
            output,
            format!(
                "offline Conda blob {} digest or size mismatch",
                descriptor.sha256
            ),
        ));
    }
    Ok(())
}

struct VerifiedInputs {
    _spool: tempfile::TempDir,
    archives: Vec<(PathBuf, String)>,
    helper: PathBuf,
    notices: BTreeMap<String, PathBuf>,
}

fn verify_inputs(
    output: &str,
    record: &PortableConda,
    blobs: &Path,
) -> Result<VerifiedInputs, ExecError> {
    record.validate()?;
    conda::admit_host_requirements(&record.lock, &NATIVE_TARGET)?;
    let spool = tempfile::Builder::new()
        .prefix("grip-conda-offline-")
        .tempdir()?;
    let mut archives = Vec::with_capacity(record.archives.len());
    for descriptor in &record.archives {
        let directory = spool.path().join(&descriptor.sha256);
        std::fs::create_dir(&directory)?;
        let mut file = File::create(directory.join("archive"))?;
        verify_copy(
            output,
            &blobs.join(&descriptor.sha256),
            descriptor,
            &mut file,
        )?;
        archives.push((directory, descriptor.sha256.clone()));
    }
    let helper = spool.path().join("helper");
    std::fs::create_dir(&helper)?;
    let path = helper.join("gripsack-conda");
    let mut file = File::create(&path)?;
    verify_copy(
        output,
        &blobs.join(&record.helper.sha256),
        &record.helper,
        &mut file,
    )?;
    file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
    let notices = notices::spool(output, &record.helper_notices, blobs, spool.path())?;
    Ok(VerifiedInputs {
        _spool: spool,
        archives,
        helper,
        notices,
    })
}

fn helper_object(home: &Path, digest: &str) -> PathBuf {
    gripsack_store::content_path(home, HELPER_NAMESPACE, digest)
}

pub(super) type PreparedConda = BTreeMap<String, conda::AdmittedArchives>;

fn prepared<'a>(
    output: &str,
    home: &Path,
    record: &PortableConda,
    inputs: &'a PreparedConda,
) -> Result<&'a conda::AdmittedArchives, ExecError> {
    let (identity, _) = conda::native_location(output, home, &record.lock)?;
    inputs.get(&identity).ok_or_else(|| {
        failure(
            output,
            "Conda originals have not passed complete closure preflight",
        )
    })
}

/// Verify original archives and derive costs without selecting or executing a
/// helper. A warm prefix is never evidence for admission of its originals.
pub(super) fn preflight(
    output: &str,
    home: &Path,
    record: &PortableConda,
    blobs: Option<&Path>,
    budget: &mut conda::AdmissionBudget,
) -> Result<conda::AdmittedArchives, ExecError> {
    record.validate()?;
    conda::admit_host_requirements(&record.lock, &NATIVE_TARGET)?;
    let mut archives = BTreeMap::new();
    for descriptor in &record.archives {
        let path = blobs.map_or_else(
            || conda::archive_object(home, &descriptor.sha256).join("archive"),
            |blobs| blobs.join(&descriptor.sha256),
        );
        verify_copy(output, &path, descriptor, &mut std::io::sink())?;
        archives.insert(descriptor.sha256.clone(), path);
    }
    conda::admit_portable_archives(output, home, &record.lock, &archives, budget)
}

/// Called only after parent signature/policy admission, under its held
/// lifecycle session. Missing/corrupt bundle inputs fail even if a matching
/// archive or helper happens to exist in the recipient's warm cache.
pub(super) fn import(
    ctx: &Ctx,
    session: &LifecycleSession,
    output: &str,
    record: &PortableConda,
    blobs: &Path,
    inputs: &PreparedConda,
) -> Result<CondaMaterialization, ExecError> {
    if session.home() != ctx.home {
        return Err(failure(output, "Conda import belongs to another home"));
    }
    let admitted = prepared(output, &ctx.home, record, inputs)?;
    let verified = verify_inputs(output, record, blobs)?;
    for ((stage, digest), descriptor) in verified.archives.iter().zip(&record.archives) {
        retain_input(
            ctx,
            output,
            stage,
            &conda::archive_object(&ctx.home, digest),
            "archive",
            descriptor,
        )?;
    }
    let object = helper_object(&ctx.home, &record.helper.sha256);
    retain_input(
        ctx,
        output,
        &verified.helper,
        &object,
        "gripsack-conda",
        &record.helper,
    )?;
    let notice_roots = notices::retain(ctx, output, &record.helper_notices, &verified.notices)?;
    let deadline = Instant::now()
        .checked_add(gripsack_process::Limits::default().timeout)
        .ok_or_else(|| failure(output, "Conda helper deadline overflow"))?;
    let helper = gripsack_conda::CondaHelper::select(
        &object.join("gripsack-conda"),
        Some(gripsack_conda::provision::approved_pin()?),
        ctx.repository.contents(),
        deadline,
    )
    .map_err(|error| failure(output, format!("offline Conda helper selection: {error}")))?;
    let mut materialized =
        conda::materialize_offline(ctx, session, output, &record.lock, &helper, admitted)?;
    materialized.artifact.retention.insert(object);
    materialized.artifact.retention.extend(notice_roots);
    Ok(materialized)
}

/// Re-admit retained materialization without helper execution or network.
/// The caller additionally rechecks the current publisher/trust policy.
pub(super) fn inspect_retained(
    output: &str,
    home: &Path,
    record: &PortableConda,
    inputs: &PreparedConda,
) -> Result<CondaMaterialization, ExecError> {
    record.validate()?;
    conda::admit_host_requirements(&record.lock, &NATIVE_TARGET)?;
    let admitted = prepared(output, home, record, inputs)?;
    let object = helper_object(home, &record.helper.sha256);
    verify_copy(
        output,
        &object.join("gripsack-conda"),
        &record.helper,
        &mut std::io::sink(),
    )?;
    let notice_roots = notices::inspect(output, home, &record.helper_notices)?;
    let mut materialized = conda::inspect_retained(output, home, &record.lock, Some(admitted))?
        .ok_or_else(|| failure(output, "retained offline Conda prefix is missing"))?;
    materialized.artifact.retention.insert(object);
    materialized.artifact.retention.extend(notice_roots);
    Ok(materialized)
}

fn retain_input(
    ctx: &Ctx,
    output: &str,
    stage: &Path,
    object: &Path,
    name: &str,
    descriptor: &BlobDescriptor,
) -> Result<(), ExecError> {
    match std::fs::symlink_metadata(object) {
        Ok(metadata) if metadata.is_dir() => {
            verify_copy(output, &object.join(name), descriptor, &mut std::io::sink())
        }
        Ok(_) => Err(failure(output, "retained Conda input is not a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            crate::source::publish(ctx, output, stage, object)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
#[path = "conda_tests.rs"]
mod tests;
