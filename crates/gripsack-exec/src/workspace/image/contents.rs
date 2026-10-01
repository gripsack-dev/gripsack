use super::{failure, plan::Placement};
use crate::ExecError;
use gripsack_buildkit::oci::{BlobDigest, FileEntry, FileKind, ValidatedImage};
use gripsack_fs::cap_std::fs::PermissionsExt;
use gripsack_ir::workspace_v6::ImageOutput;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, io::Read, ops::Bound, path::Path};

pub(super) fn validate(
    image: &ImageOutput,
    placements: &[Placement<'_, '_>],
    actual: &ValidatedImage,
) -> Result<(), ExecError> {
    let mut expected_paths = BTreeSet::from(["/".to_owned()]);
    let mut buffer = [0; 64 * 1024];
    for placement in placements {
        let prefix = placement.destination.path.as_str();
        let owner = placement.destination.owner;
        expected_paths.insert(prefix.to_owned());
        let expected_root = FileEntry { kind:FileKind::Directory, mode:0o755, uid:owner.uid, gid:owner.gid };
        if actual.files().get(prefix) != Some(&expected_root) {
            return Err(failure(image, format!("image destination for {:?} differs in kind/mode/ownership",placement.name)));
        }
        let root = gripsack_fs::open(&placement.package.producer.payload)?;
        let mut pending = vec![(root,String::new())];
        while let Some((directory, relative)) = pending.pop() {
            for entry in directory.entries()? {
                let entry = entry?;
                let name = entry.file_name();
                let component = name.to_str().ok_or_else(|| failure(image,"package path is not UTF-8"))?;
                let relative = if relative.is_empty() { component.to_owned() } else { format!("{relative}/{component}") };
                let image_path = format!("{prefix}/{relative}");
                let metadata = directory.symlink_metadata(&name)?;
                let kind = metadata.file_type();
                let (kind,mode) = if kind.is_dir() {
                    pending.push((gripsack_fs::open_dir_nofollow(&directory,Path::new(&name))?,relative));
                    (FileKind::Directory,0o755)
                } else if kind.is_symlink() {
                    let target = directory.read_link_contents(&name)?;
                    let target = target.to_str().ok_or_else(|| failure(image,"package link target is not UTF-8"))?;
                    (FileKind::Symlink{target:target.to_owned()},0o777)
                } else if kind.is_file() {
                    let mut file = gripsack_fs::open_file_nofollow(&directory,Path::new(&name))?;
                    let mut hash = Sha256::new();
                    let mut size = 0u64;
                    loop {
                        let count = file.read(&mut buffer)?;
                        if count == 0 { break; }
                        size = size.checked_add(count as u64).ok_or_else(|| failure(image,"package byte count overflow"))?;
                        if size > metadata.len() { return Err(failure(image,"package file changed while comparing OCI bytes")); }
                        hash.update(&buffer[..count]);
                    }
                    if size != metadata.len() { return Err(failure(image,"package file changed while comparing OCI bytes")); }
                    let mode = if metadata.permissions().mode() & 0o111 != 0 { 0o755 } else { 0o644 };
                    (FileKind::File{size,digest:BlobDigest::from_bytes(hash.finalize().into())},mode)
                } else {
                    return Err(failure(image,"unsupported special file in a runtime package"));
                };
                let expected = FileEntry { kind,mode,uid:owner.uid,gid:owner.gid };
                if actual.files().get(&image_path) != Some(&expected) {
                    return Err(failure(image,format!("OCI file {image_path:?} does not match its retained package bytes/link/mode/owner")));
                }
                expected_paths.insert(image_path);
            }
        }
        let upper = format!("{prefix}0");
        for (path, _) in actual.files().range::<str,_>((Bound::Included(prefix),Bound::Excluded(upper.as_str()))) {
            if (path == prefix || path.strip_prefix(prefix).is_some_and(|suffix| suffix.starts_with('/'))) && !expected_paths.contains(path) {
                return Err(failure(image,format!("unexpected base or exporter content inside package destination {path:?}")));
            }
        }
    }
    if image.base.is_none() {
        let mut parents = BTreeSet::new();
        for path in &expected_paths {
            let mut path = path.as_str();
            while let Some((parent,_)) = path.rsplit_once('/') {
                if parent.is_empty() { break; }
                parents.insert(parent.to_owned());
                path = parent;
            }
        }
        for (path, entry) in actual.files() {
            if !expected_paths.contains(path) && !(parents.contains(path) && matches!(entry.kind,FileKind::Directory)) {
                return Err(failure(image,format!("unselected content in scratch image: {path:?}")));
            }
        }
    }
    Ok(())
}
