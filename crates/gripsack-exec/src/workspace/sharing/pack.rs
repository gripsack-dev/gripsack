//! Explicit publisher operation. Capture trees through the existing bounded
//! acquisition adapter before hashing/signing; never read an installed Conda
//! prefix as a supposedly relocatable payload.
use super::format::{self, BlobDescriptor, EntryKind, PortableEntry, PortableManifest};
use crate::{Ctx, ExecError, LifecycleSession};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
};

pub fn pack(
    ctx: &Ctx,
    session: &LifecycleSession,
    specification: &[u8],
    source: &Path,
    output: &Path,
    key: &Path,
    key_id: &str,
) -> Result<String, ExecError> {
    if specification.len() > format::MAX_MANIFEST_BYTES {
        return Err(format::failure("package specification exceeds byte bound"));
    }
    let mut manifest: PortableManifest = serde_json::from_slice(specification)?;
    if session.home() != ctx.home {
        return Err(format::failure("package capture belongs to another home"));
    }
    manifest.target.validate()?;
    if output.exists() {
        return Err(format::failure("bundle output already exists"));
    }
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let staging = tempfile::Builder::new()
        .prefix(".grip-package-")
        .tempdir_in(parent)?;
    let blobs = staging.path().join("blobs");
    std::fs::create_dir(&blobs)?;
    std::fs::create_dir(staging.path().join("manifests"))?;
    let mut payload_bytes = 0u64;
    let mut metadata_bytes = 0usize;
    if manifest.components.len() > format::MAX_COMPONENTS {
        return Err(format::failure("component count exceeds bound"));
    }
    for component in &mut manifest.components {
        if !format::identifier(&component.id) {
            return Err(format::failure("invalid component ID"));
        }
        if let Some(external) = &component.external {
            if component.payload.is_some()
                || component.tree.is_some()
                || component.conda.is_some()
                || !component.inventory.is_empty()
            {
                return Err(format::failure(
                    "external component cannot carry a copied local payload",
                ));
            }
            merge_external(
                session,
                external,
                staging.path(),
                &mut payload_bytes,
                &mut metadata_bytes,
            )?;
            continue;
        }
        if let Some(conda) = &component.conda {
            // Export inventory binds original CAS archives and the core-approved
            // helper, not publisher-selected bootstrap executables.
            let (record, files) = super::conda::export_inventory(ctx, "package", &conda.lock)?;
            for (digest, path) in files {
                copy_blob(&path, &blobs.join(digest), &mut payload_bytes)?;
            }
            component.conda = Some(record);
            component.payload = None;
            component.tree = None;
            component.inventory.clear();
            continue;
        }
        let capture = tempfile::Builder::new()
            .prefix("grip-package-capture-")
            .tempdir()?;
        let tree = capture.path().join("tree");
        let input = source.join("components").join(&component.id);
        let path = input
            .to_str()
            .ok_or_else(|| format::failure("package source is not UTF-8"))?;
        ctx.fetch.fetch(
            &gripsack_ir::FetchSpec::File { path: path.into() },
            &tree,
            None,
        )?;
        let mut inventory = Vec::new();
        inventory_tree(&tree, &tree, &mut inventory)?;
        // Generic local acquisition creates directories with its safe defaults.
        // Preserve explicitly selected publisher directory modes in this
        // private snapshot before deriving the signed inventory/tree identity.
        let source_directory = gripsack_fs::open(&input)?;
        let captured_directory = gripsack_fs::open(&tree)?;
        for entry in inventory.iter_mut().rev() {
            if matches!(entry.kind, EntryKind::Directory) {
                let relative = Path::new(&entry.path);
                let original =
                    gripsack_fs::open_dir_nofollow(&source_directory, relative)?.into_std_file();
                entry.mode = original.metadata()?.permissions().mode() & 0o7777;
                if entry.mode & !0o777 != 0 || entry.mode & 0o022 != 0 {
                    return Err(format::failure(
                        "portable directories reject privileged or group/world-writable modes",
                    ));
                }
                gripsack_fs::open_dir_nofollow(&captured_directory, relative)?
                    .into_std_file()
                    .set_permissions(std::fs::Permissions::from_mode(entry.mode))?;
            }
        }
        inventory.sort_by(|left, right| left.path.cmp(&right.path));
        // The capture may live on another filesystem. Write transfer bytes
        // beside their destination for atomic no-clobber publication.
        let mut temporary = tempfile::NamedTempFile::new_in(&blobs)?;
        let mut archive = tar::Builder::new(temporary.as_file_mut());
        archive.follow_symlinks(false);
        for entry in &inventory {
            let mut header = tar::Header::new_gnu();
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(0);
            header.set_mode(entry.mode);
            match &entry.kind {
                EntryKind::File { size, .. } => {
                    header.set_size(*size);
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_cksum();
                    archive.append_data(
                        &mut header,
                        &entry.path,
                        std::fs::File::open(tree.join(&entry.path))?,
                    )?;
                }
                EntryKind::Directory => {
                    header.set_size(0);
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_cksum();
                    archive.append_data(&mut header, &entry.path, std::io::empty())?;
                }
                EntryKind::Symlink { target } => {
                    header.set_size(0);
                    header.set_entry_type(tar::EntryType::Symlink);
                    header.set_link_name(target)?;
                    header.set_cksum();
                    archive.append_data(&mut header, &entry.path, std::io::empty())?;
                }
            }
        }
        archive.finish()?;
        archive.into_inner()?.sync_all()?;
        let (sha256, size) = hash_file(temporary.path())?;
        let destination = blobs.join(&sha256);
        if !destination.exists() {
            add_payload_bytes(&mut payload_bytes, size)?;
            drop(
                temporary
                    .persist_noclobber(&destination)
                    .map_err(|error| error.error)?,
            );
        }
        component.payload = Some(BlobDescriptor {
            sha256,
            size,
            media_type: format::TREE_MEDIA_TYPE.into(),
        });
        component.tree = Some(gripsack_store::canonical_tree_hash(&tree)?.to_string());
        component.inventory = inventory;
    }
    let bytes = manifest.encode()?;
    let envelope = super::trust::sign(&bytes, key, key_id)
        .map_err(|error| format::failure(error.to_string()))?;
    metadata_bytes = metadata_bytes
        .checked_add(bytes.len())
        .and_then(|n| n.checked_add(envelope.len()))
        .ok_or_else(|| format::failure("package metadata byte count overflow"))?;
    if metadata_bytes > super::closure::MAX_CLOSURE_METADATA_BYTES {
        return Err(format::failure(
            "complete package metadata exceeds byte bound",
        ));
    }
    write_synced(&staging.path().join("manifest.json"), &bytes)?;
    write_synced(&staging.path().join("envelope.json"), &envelope)?;
    super::closure::validate_directory(staging.path())?;
    // Output is user-selected transfer material, not an admitted local receipt.
    // A subsequent import still performs all trust and runtime admission.
    gripsack_fs::publish_dir_at(staging.path(), output)?;
    manifest.digest()
}
pub(super) fn inventory_tree(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<PortableEntry>,
) -> Result<(), ExecError> {
    for child in std::fs::read_dir(directory)? {
        let child = child?;
        let path = child.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| format::failure("inventory escaped root"))?;
        let text = relative
            .to_str()
            .ok_or_else(|| format::failure("inventory path is not UTF-8"))?
            .to_owned();
        if !format::relative_path(&text) {
            return Err(format::failure("unsupported portable path"));
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        let kind = if metadata.is_dir() {
            EntryKind::Directory
        } else if metadata.is_file() {
            let (sha256, size) = hash_file(&path)?;
            EntryKind::File { sha256, size }
        } else if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&path)?;
            EntryKind::Symlink {
                target: target
                    .to_str()
                    .ok_or_else(|| format::failure("link is not UTF-8"))?
                    .into(),
            }
        } else {
            return Err(format::failure("special files are not portable payloads"));
        };
        // Symlink permissions have no Unix runtime semantics; canonicalize to
        // 0755 rather than admitting the kernel's apparent 0777 as writable.
        let mode = if metadata.file_type().is_symlink() {
            0o755
        } else {
            metadata.permissions().mode() & 0o7777
        };
        entries.push(PortableEntry {
            path: text,
            mode,
            kind,
        });
        if entries.len() > format::MAX_ENTRIES {
            return Err(format::failure("portable inventory exceeds entry bound"));
        }
        if metadata.is_dir() {
            inventory_tree(root, &path, entries)?;
        }
    }
    Ok(())
}
pub(super) fn hash_file(path: &Path) -> Result<(String, u64), ExecError> {
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| format::failure("blob size overflow"))?;
        if size > format::MAX_PAYLOAD_BYTES {
            return Err(format::failure("blob exceeds byte bound"));
        }
        digest.update(&buffer[..count]);
    }
    Ok((
        gripsack_process::Sha256Digest::from_bytes(digest.finalize().into()).to_string(),
        size,
    ))
}
fn copy_blob(source: &Path, destination: &Path, total: &mut u64) -> Result<(), ExecError> {
    let (digest, size) = hash_file(source)?;
    if destination.file_name().and_then(|name| name.to_str()) != Some(digest.as_str()) {
        return Err(format::failure("original archive changed during export"));
    }
    if destination.exists() {
        return Ok(());
    }
    add_payload_bytes(total, size)?;
    std::fs::copy(source, destination)?;
    std::fs::File::open(destination)?.sync_all()?;
    Ok(())
}
fn add_payload_bytes(total: &mut u64, size: u64) -> Result<(), ExecError> {
    *total = total
        .checked_add(size)
        .ok_or_else(|| format::failure("closure payload count overflow"))?;
    if *total > format::MAX_PAYLOAD_BYTES {
        return Err(format::failure(
            "complete closure exceeds payload byte bound",
        ));
    }
    Ok(())
}

fn merge_external(
    session: &LifecycleSession,
    reference: &format::ExternalReference,
    staging: &Path,
    payload_bytes: &mut u64,
    metadata_bytes: &mut usize,
) -> Result<(), ExecError> {
    format::digest(&reference.manifest)?;
    let temporary = tempfile::Builder::new()
        .prefix("grip-package-external-")
        .tempdir()?;
    let bundle = temporary.path().join("bundle");
    let facts = super::runtime::export_in_session(session, &reference.manifest, &bundle)?;
    if facts.namespace != reference.namespace || facts.publisher != reference.publisher {
        return Err(format::failure(
            "external manifest namespace/publisher differs from selected authority",
        ));
    }
    merge_manifest(&bundle, &reference.manifest, staging, metadata_bytes)?;
    let externals = bundle.join("manifests");
    if externals.exists() {
        for entry in std::fs::read_dir(externals)? {
            let entry = entry?;
            let identity = entry
                .file_name()
                .into_string()
                .map_err(|_| format::failure("external manifest name is not UTF-8"))?;
            format::digest(&identity)?;
            merge_manifest(&entry.path(), &identity, staging, metadata_bytes)?;
        }
    }
    for entry in std::fs::read_dir(bundle.join("blobs"))? {
        let entry = entry?;
        let identity = entry
            .file_name()
            .into_string()
            .map_err(|_| format::failure("blob name is not UTF-8"))?;
        format::digest(&identity)?;
        copy_blob(
            &entry.path(),
            &staging.join("blobs").join(identity),
            payload_bytes,
        )?;
    }
    Ok(())
}

fn merge_manifest(
    source: &Path,
    identity: &str,
    staging: &Path,
    total: &mut usize,
) -> Result<(), ExecError> {
    let directory = gripsack_fs::open(source)?;
    let destination = staging.join("manifests").join(identity);
    let exists = destination.exists();
    if !exists {
        if std::fs::read_dir(staging.join("manifests"))?.count()
            >= super::closure::MAX_CLOSURE_MANIFESTS - 1
        {
            return Err(format::failure(
                "complete package exceeds manifest count bound",
            ));
        }
        std::fs::create_dir(&destination)?;
    }
    for (name, limit) in [
        ("manifest.json", format::MAX_MANIFEST_BYTES),
        ("envelope.json", super::trust::MAX_ENVELOPE_BYTES),
    ] {
        let bytes = super::receipt::read(&directory, Path::new(name), limit as u64)?;
        if exists {
            let prior = gripsack_fs::open(&destination)?;
            if super::receipt::read(&prior, Path::new(name), limit as u64)? != bytes {
                return Err(format::failure(
                    "conflicting exact metadata for one external manifest",
                ));
            }
        } else {
            *total = total
                .checked_add(bytes.len())
                .ok_or_else(|| format::failure("closure metadata count overflow"))?;
            if *total > super::closure::MAX_CLOSURE_METADATA_BYTES {
                return Err(format::failure("closure metadata exceeds byte bound"));
            }
            write_synced(&destination.join(name), &bytes)?;
        }
    }
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), ExecError> {
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
