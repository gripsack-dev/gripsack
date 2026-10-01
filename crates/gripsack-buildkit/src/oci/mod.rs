//! Independent OCI intake. BuildKit owns assembly; this module only decodes and
//! verifies bounded bytes, descriptors, DiffIDs, runtime configuration and the
//! resulting filesystem. Completion messages never construct this capability.
mod archive;
mod digest;
mod file_region;
mod layer;
mod model;
mod runtime;
#[cfg(test)]
mod tests;

pub use digest::BlobDigest;
use crate::plan::{ImageConfig, Platform};
use std::{collections::{BTreeMap, BTreeSet}, fs::File};

#[derive(Debug, Clone, Copy)]
pub struct OciLimits {
    pub archive_bytes: u64,
    pub expanded_bytes: u64,
    pub entries: usize,
    pub metadata_bytes: u64,
}
impl Default for OciLimits {
    fn default() -> Self {
        Self { archive_bytes:8 * 1024 * 1024 * 1024, expanded_bytes:4 * 1024 * 1024 * 1024, entries:100_000, metadata_bytes:128 * 1024 * 1024 }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum OciError {
    #[error("OCI I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("OCI metadata cannot be decoded: {0}")]
    Json(#[from] serde_json::Error),
    #[error("OCI artifact rejected: {0}")]
    Invalid(&'static str),
    #[error("OCI runtime {path:?}: {detail}")]
    Runtime { path: String, detail: String },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileKind {
    File { size: u64, digest: BlobDigest },
    Directory,
    Symlink { target: String },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub kind: FileKind,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

pub struct ValidatedImage {
    manifest: BlobDigest,
    configuration: BlobDigest,
    layers: Vec<BlobDigest>,
    files: BTreeMap<String, FileEntry>,
}
impl ValidatedImage {
    pub fn manifest(&self) -> BlobDigest { self.manifest }
    pub fn configuration(&self) -> BlobDigest { self.configuration }
    pub fn layers(&self) -> &[BlobDigest] { &self.layers }
    pub fn files(&self) -> &BTreeMap<String, FileEntry> { &self.files }
}

pub fn validate(
    file: &mut File,
    platform: Platform,
    expected: &ImageConfig,
    limits: OciLimits,
) -> Result<ValidatedImage, OciError> {
    expected.validate().map_err(OciError::Invalid)?;
    let archive = archive::ArchiveIndex::scan(file, limits)?;
    const JSON_BYTES: u64 = 1024 * 1024;
    let json_limit = limits.metadata_bytes.min(JSON_BYTES);
    let layout: model::Layout = archive::json(file, archive.layout, json_limit)?;
    if layout.version != "1.0.0" { return Err(OciError::Invalid("unsupported OCI layout version")); }
    let index: model::Index = archive::json(file, archive.index, json_limit)?;
    if index.version != 2 || index.media_type.as_deref().is_some_and(|value| value != model::INDEX_MEDIA) || index.manifests.len() != 1 {
        return Err(OciError::Invalid("OCI index must select exactly one supported image manifest"));
    }
    annotations(&index.annotations, None)?;
    let selected = &index.manifests[0];
    descriptor_platform(selected, platform)?;
    annotations(&selected.annotations, None)?;
    let manifest: model::Manifest = archive::json(file, archive.descriptor(selected, model::MANIFEST_MEDIA)?, json_limit)?;
    if manifest.version != 2 || manifest.media_type != model::MANIFEST_MEDIA || manifest.layers.len() > 2048 {
        return Err(OciError::Invalid("unsupported OCI manifest version/media type or layer count"));
    }
    annotations(&manifest.annotations, None)?;
    descriptor_platform(&manifest.config, platform)?;
    annotations(&manifest.config.annotations, None)?;
    let configuration: model::Configuration = archive::json(file, archive.descriptor(&manifest.config, model::CONFIG_MEDIA)?, json_limit)?;
    if configuration.os != "linux" || configuration.architecture != platform.architecture.as_str() || configuration.variant.as_deref().is_some_and(|value| !value.is_empty()) || configuration.created != model::EPOCH {
        return Err(OciError::Invalid("OCI image platform or creation epoch differs from the admitted plan"));
    }
    let actual = &configuration.config;
    if actual.entrypoint != expected.entrypoint || actual.args != expected.args || actual.env != expected.env || actual.working_dir != expected.cwd || actual.user != expected.user {
        return Err(OciError::Invalid("OCI runtime configuration differs from the checked exporter"));
    }
    if configuration.rootfs.kind != "layers" || configuration.rootfs.diff_ids.len() != manifest.layers.len() || configuration.history.len() > 4096 {
        return Err(OciError::Invalid("OCI rootfs/history does not describe its layer sequence"));
    }
    if configuration.history.iter().any(|entry| entry.created != model::EPOCH || entry.created_by.len() > 4096 || entry.comment.len() > 4096)
        || configuration.history.iter().filter(|entry| !entry.empty_layer).count() != manifest.layers.len() {
        return Err(OciError::Invalid("OCI history differs from its fixed epoch/layer sequence"));
    }
    let mut used = BTreeSet::from([selected.digest, manifest.config.digest]);
    let mut files = BTreeMap::from([("/".into(), FileEntry { kind:FileKind::Directory, mode:0o755, uid:0, gid:0 })]);
    let mut budget = layer::Budget::new(limits);
    let mut layers = Vec::with_capacity(manifest.layers.len());
    let mut metadata = runtime::MetadataIndex::new();
    for (descriptor, diff) in manifest.layers.iter().zip(&configuration.rootfs.diff_ids) {
        let region = archive.descriptor(descriptor, model::LAYER_MEDIA)?;
        descriptor_platform(descriptor, platform)?;
        annotations(&descriptor.annotations, Some(*diff))?;
        used.insert(descriptor.digest);
        layer::apply(file, region, *diff, &mut budget, &mut files, &mut metadata)?;
        layers.push(descriptor.digest);
    }
    if used.len() != archive.blobs.len() { return Err(OciError::Invalid("OCI archive contains unreferenced payload blobs")); }
    let image = ValidatedImage { manifest:selected.digest, configuration:manifest.config.digest, layers, files };
    if !matches!(resolve(&image.files, &expected.cwd)?.1.kind, FileKind::Directory) {
        return Err(OciError::Invalid("OCI working directory is absent or not a directory"));
    }
    runtime::validate(&image.files, &metadata, platform, expected)?;
    Ok(image)
}

fn descriptor_platform(descriptor: &model::Descriptor, expected: Platform) -> Result<(), OciError> {
    if descriptor.platform.as_ref().is_some_and(|platform| platform.os != "linux" || platform.architecture != expected.architecture.as_str() || platform.variant.as_deref().is_some_and(|value| !value.is_empty()) || platform.os_version.as_deref().is_some_and(|value| !value.is_empty()) || !platform.os_features.is_empty()) {
        return Err(OciError::Invalid("OCI descriptor platform substitution"));
    }
    // Layer-specific annotations are checked with their independent DiffID.
    for (key, value) in &descriptor.annotations {
        if key == "org.opencontainers.image.created" && value != model::EPOCH { return Err(OciError::Invalid("OCI descriptor timestamp substitution")); }
    }
    Ok(())
}
fn annotations(values: &BTreeMap<String, String>, diff: Option<BlobDigest>) -> Result<(), OciError> {
    for (key, value) in values {
        let valid = match key.as_str() {
            "org.opencontainers.image.created" => value == model::EPOCH,
            "buildkit/rewritten-timestamp" => diff.is_some() && value == "0",
            "containerd.io/uncompressed" => diff.is_some_and(|expected| BlobDigest::parse(value).is_ok_and(|actual| actual == expected)),
            _ => false,
        };
        if !valid { return Err(OciError::Invalid("undeclared or substituted OCI annotation")); }
    }
    Ok(())
}

/// Resolve links inside the image namespace, never against the host filesystem.
fn resolve<'a>(files: &'a BTreeMap<String, FileEntry>, path: &str) -> Result<(&'a str, &'a FileEntry), OciError> {
    if !path.starts_with('/') { return Err(OciError::Invalid("OCI runtime path must be absolute")); }
    let mut path = path.to_owned();
    for _ in 0..40 {
        let mut current = String::new();
        let mut redirected = None;
        let mut parts = path[1..].split('/').peekable();
        while let Some(part) = parts.next() {
            if part.is_empty() { continue; }
            current.push('/'); current.push_str(part);
            let entry = files.get(&current).ok_or(OciError::Invalid("OCI runtime path is missing"))?;
            if let FileKind::Symlink { target } = &entry.kind {
                let parent = current.rsplit_once('/').map_or("/", |(prefix, _)| prefix);
                let joined = if target.starts_with('/') { target.clone() } else { format!("{parent}/{target}") };
                let mut components = Vec::new();
                for component in joined.split('/').chain(parts) {
                    match component { "" | "." => {}, ".." => { if components.pop().is_none() { return Err(OciError::Invalid("OCI symlink escapes the image root")); } }, value => components.push(value) }
                }
                redirected = Some(format!("/{}", components.join("/")));
                break;
            }
            if parts.peek().is_some() && !matches!(entry.kind, FileKind::Directory) { return Err(OciError::Invalid("OCI runtime parent is not a directory")); }
        }
        if let Some(next) = redirected { path = next; } else {
            return files.get_key_value(&path).map(|(path,entry)| (path.as_str(),entry)).ok_or(OciError::Invalid("OCI runtime path is missing"));
        }
    }
    Err(OciError::Invalid("OCI runtime symlink cycle or depth limit"))
}
