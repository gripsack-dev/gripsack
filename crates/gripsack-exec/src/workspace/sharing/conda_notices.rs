//! Core-approved helper redistribution notices. Crate-local copies preserve
//! standalone Cargo packaging; helper release packaging checks them against
//! canonical project/vendor sources. Publishers cannot replace this inventory.
use super::{BlobDescriptor, failure, retain_input, verify_copy};
use crate::{Ctx, ExecError};
use gripsack_process::Sha256Digest;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    path::{Path, PathBuf},
    sync::LazyLock,
};

const MEDIA_TYPE: &str = "application/vnd.gripsack.conda.helper-notice.v1";
const NAMESPACE: &str = "portable-conda-notice";
const ORIGINALS: &[(&str, &[u8])] = &[
    (
        "LICENSE-gripsack",
        include_bytes!("notices/LICENSE-gripsack"),
    ),
    ("LICENSE-pixi", include_bytes!("notices/LICENSE-pixi")),
    ("LICENSE-UV-MIT", include_bytes!("notices/LICENSE-UV-MIT")),
    ("LICENSE-rattler", include_bytes!("notices/LICENSE-rattler")),
];

static DESCRIPTORS: LazyLock<BTreeMap<String, BlobDescriptor>> = LazyLock::new(|| {
    ORIGINALS
        .iter()
        .map(|(name, bytes)| {
            (
                name.to_string(),
                BlobDescriptor {
                    sha256: Sha256Digest::of(bytes).to_string(),
                    size: bytes.len() as u64,
                    media_type: MEDIA_TYPE.into(),
                },
            )
        })
        .collect()
});

pub(super) fn descriptors() -> &'static BTreeMap<String, BlobDescriptor> {
    &DESCRIPTORS
}

fn object(home: &Path, digest: &str) -> PathBuf {
    gripsack_store::content_path(home, NAMESPACE, digest)
}

/// Export from compiled metadata, never incidental publisher files. Identical
/// notice bytes (Pixi/Rattler) share a transport blob while preserving labels.
pub(super) fn export(ctx: &Ctx, output: &str) -> Result<BTreeMap<String, PathBuf>, ExecError> {
    let mut paths = BTreeMap::new();
    for (name, bytes) in ORIGINALS {
        let descriptor = &descriptors()[*name];
        if paths.contains_key(&descriptor.sha256) {
            continue;
        }
        let stage = tempfile::Builder::new()
            .prefix("grip-conda-notice-")
            .tempdir()?;
        std::fs::write(stage.path().join("notice"), bytes)?;
        let destination = object(&ctx.home, &descriptor.sha256);
        retain_input(
            ctx,
            output,
            stage.path(),
            &destination,
            "notice",
            descriptor,
        )?;
        paths.insert(descriptor.sha256.clone(), destination.join("notice"));
    }
    Ok(paths)
}

/// Verify every signed notice before the parent publishes any bundle input.
pub(super) fn spool(
    output: &str,
    required: &BTreeMap<String, BlobDescriptor>,
    blobs: &Path,
    staging: &Path,
) -> Result<BTreeMap<String, PathBuf>, ExecError> {
    let mut paths = BTreeMap::new();
    for descriptor in required.values() {
        if paths.contains_key(&descriptor.sha256) {
            continue;
        }
        let directory = staging.join(format!("notice-{}", descriptor.sha256));
        std::fs::create_dir(&directory)?;
        let mut file = File::create(directory.join("notice"))?;
        verify_copy(
            output,
            &blobs.join(&descriptor.sha256),
            descriptor,
            &mut file,
        )?;
        paths.insert(descriptor.sha256.clone(), directory);
    }
    Ok(paths)
}

pub(super) fn retain(
    ctx: &Ctx,
    output: &str,
    required: &BTreeMap<String, BlobDescriptor>,
    spools: &BTreeMap<String, PathBuf>,
) -> Result<BTreeSet<PathBuf>, ExecError> {
    let mut roots = BTreeSet::new();
    for descriptor in required.values() {
        let destination = object(&ctx.home, &descriptor.sha256);
        if roots.insert(destination.clone()) {
            let stage = spools
                .get(&descriptor.sha256)
                .ok_or_else(|| failure(output, "verified helper notice spool is missing"))?;
            retain_input(ctx, output, stage, &destination, "notice", descriptor)?;
        }
    }
    Ok(roots)
}

pub(super) fn inspect(
    output: &str,
    home: &Path,
    required: &BTreeMap<String, BlobDescriptor>,
) -> Result<BTreeSet<PathBuf>, ExecError> {
    let mut roots = BTreeSet::new();
    for descriptor in required.values() {
        let destination = object(home, &descriptor.sha256);
        if roots.insert(destination.clone()) {
            verify_copy(
                output,
                &destination.join("notice"),
                descriptor,
                &mut std::io::sink(),
            )?;
        }
    }
    Ok(roots)
}
