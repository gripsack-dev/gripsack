//! Copied-byte capture and read-only sealing through pinned capabilities.
use super::inventory::{MAX_DEPTH, MAX_FILE_BYTES, invalid};
use super::policy::CaptureAdmission;
use super::{
    CaptureBudget, CaptureRoot, SourceEntry, SourceFileBytes, SourceInventory, SourceObject,
    SourceRootKind,
};
use crate::prior::FileMode;
use cap_std::fs::{MetadataExt, PermissionsExt};
use gripsack_fs::{
    Dir, cap_std,
    fault::{Boundary, operation},
};
use gripsack_process::Sha256Digest;
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

struct FileCapture<'a> {
    file: cap_std::fs::File,
    budget: &'a mut CaptureBudget,
    path: &'a Path,
    bytes: u64,
    digest: Sha256,
}

impl Read for FileCapture<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        // One excess byte is enough to refuse a file which grows after stat;
        // the publication helper never receives that over-budget byte.
        let remaining = MAX_FILE_BYTES - self.bytes;
        let length = output
            .len()
            .min(usize::try_from(remaining + 1).unwrap_or(usize::MAX));
        let count = operation(Boundary::Read, self.path, || {
            self.file.read(&mut output[..length])
        })?;
        let count_u64 =
            u64::try_from(count).map_err(|_| invalid("source read length exceeds its domain"))?;
        let total = self
            .bytes
            .checked_add(count_u64)
            .ok_or_else(|| invalid("source file size overflow"))?;
        if total > MAX_FILE_BYTES {
            return Err(invalid("source file exceeds its byte budget"));
        }
        self.budget.content(count)?;
        self.digest.update(&output[..count]);
        self.bytes = total;
        Ok(count)
    }
}

struct Capture<'a> {
    roots: &'a [CaptureRoot],
    destination: &'a Dir,
    admission: &'a CaptureAdmission<'a>,
    budget: &'a mut CaptureBudget,
    entries: Vec<SourceEntry>,
    exclusions: Vec<String>,
}

fn logical_name(path: &Path) -> io::Result<&str> {
    path.to_str()
        .ok_or_else(|| invalid("source paths must be UTF-8"))
}

fn require_absent(directory: &Dir, path: &Path) -> io::Result<()> {
    match directory.symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "source names collide on the capture filesystem",
        )),
    }
}

fn relative_target(parent: &Path, target: &Path) -> io::Result<PathBuf> {
    let from: Vec<_> = parent.components().collect();
    let to: Vec<_> = target.components().collect();
    if from
        .iter()
        .chain(&to)
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid("captured alias paths must be root-relative"));
    }
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    let mut result = PathBuf::new();
    for _ in common..from.len() {
        result.push("..");
    }
    for component in &to[common..] {
        result.push(component.as_os_str());
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    Ok(result)
}

impl Capture<'_> {
    fn alias(&mut self, path: PathBuf, target: PathBuf) -> io::Result<()> {
        let target_name = logical_name(&target)?;
        self.budget.metadata_bytes(target_name.len())?;
        let relative = relative_target(
            path.parent()
                .ok_or_else(|| invalid("alias has no parent"))?,
            &target,
        )?;
        self.destination.symlink_contents(&relative, &path)?;
        self.entries.push(SourceEntry {
            path: logical_name(&path)?.to_owned(),
            object: SourceObject::Alias {
                target: target_name.to_owned(),
            },
        });
        Ok(())
    }

    fn directory(&mut self, root_index: usize, source: &Dir, relative: &Path) -> io::Result<()> {
        for child in source.read_dir(".")? {
            let child = child?;
            let name = child.file_name();
            if name.to_str().is_none() {
                return Err(invalid("source paths must be UTF-8"));
            }
            let source_relative = relative.join(&name);
            let root = &self.roots[root_index];
            let logical = root.logical(&source_relative);
            if logical.components().count() > MAX_DEPTH {
                return Err(invalid("source tree exceeds its path-depth budget"));
            }
            let name_in_inventory = logical_name(&logical)?;
            self.budget.entry(name_in_inventory)?;
            let physical = root.canonical.join(&source_relative);
            if self.admission.excludes(root, &source_relative) {
                self.exclusions.push(name_in_inventory.to_owned());
                continue;
            }
            require_absent(self.destination, &logical)?;
            // A selected source SDK must keep its realpath outside node_modules
            // so Deno can type-strip it, including a linked node_modules parent.
            if root.kind != SourceRootKind::PinnedFrontend
                && self.roots.iter().any(|candidate| {
                    candidate.kind == SourceRootKind::PinnedFrontend
                        && physical == candidate.canonical
                })
            {
                self.alias(logical, PathBuf::from("pin"))?;
                continue;
            }
            let metadata = source.symlink_metadata(Path::new(&name))?;
            if root.kind == SourceRootKind::Repository
                && self.admission.policy.requires_directory(&source_relative)
                && !metadata.is_dir()
            {
                return Err(invalid(&format!(
                    "capture exclusion ancestor {} must remain a real directory",
                    logical.display()
                )));
            }
            if metadata.file_type().is_symlink() {
                let target = source.read_link_contents(Path::new(&name))?;
                let target = super::resolve::resolve(
                    self.roots,
                    root_index,
                    relative,
                    &target,
                    self.admission,
                    self.budget,
                )
                .map_err(|error| {
                    io::Error::new(
                        error.kind(),
                        format!("source alias {}: {error}", logical.display()),
                    )
                })?;
                self.alias(logical, target)?;
            } else if metadata.is_dir() {
                let directory = gripsack_fs::open_dir_nofollow(source, Path::new(&name))?;
                self.destination.create_dir(&logical)?;
                self.entries.push(SourceEntry {
                    path: name_in_inventory.to_owned(),
                    object: SourceObject::Directory,
                });
                self.directory(root_index, &directory, &source_relative)?;
            } else if metadata.is_file() {
                let file = gripsack_fs::open_file_nofollow(source, Path::new(&name))?;
                let metadata = file.metadata()?;
                let mode = FileMode::try_from(metadata.mode() & 0o7777)?;
                if metadata.len() > MAX_FILE_BYTES {
                    return Err(invalid("source file exceeds its byte budget"));
                }
                let mut copied = FileCapture {
                    file,
                    budget: self.budget,
                    path: &logical,
                    bytes: 0,
                    digest: Sha256::new(),
                };
                let readonly_mode = if mode.bits() & 0o111 == 0 {
                    0o400
                } else {
                    0o500
                };
                let mut options = cap_std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                let mut target = self.destination.open_with(&logical, &options)?;
                io::copy(&mut copied, &mut target)?;
                target.set_permissions(cap_std::fs::Permissions::from_mode(readonly_mode))?;
                let bytes = SourceFileBytes::try_from(copied.bytes)?;
                let sha256 = Sha256Digest::from_bytes(copied.digest.finalize().into());
                self.entries.push(SourceEntry {
                    path: name_in_inventory.to_owned(),
                    object: SourceObject::File {
                        bytes,
                        sha256,
                        mode,
                    },
                });
            } else {
                return Err(invalid("source capture refuses special files"));
            }
        }
        Ok(())
    }
}

pub(super) fn copy_roots(
    roots: &[CaptureRoot],
    destination: &Dir,
    admission: &CaptureAdmission<'_>,
    budget: &mut CaptureBudget,
) -> io::Result<SourceInventory> {
    let mut capture = Capture {
        roots,
        destination,
        admission,
        budget,
        entries: Vec::new(),
        exclusions: Vec::new(),
    };
    for (index, root) in roots.iter().enumerate() {
        let name = root.kind.directory();
        capture.budget.entry(name)?;
        require_absent(destination, Path::new(name))?;
        destination.create_dir(name)?;
        capture.entries.push(SourceEntry {
            path: name.to_owned(),
            object: SourceObject::Directory,
        });
        capture.directory(index, &root.directory, Path::new(""))?;
    }
    let inventory = SourceInventory::new(
        roots.iter().map(|root| root.kind).collect(),
        capture.entries,
        capture.exclusions,
    )?;
    if roots
        .iter()
        .any(|root| root.kind == SourceRootKind::PinnedFrontend)
    {
        if destination.canonicalize("repo/node_modules/@gripsack/core")? != Path::new("pin") {
            return Err(invalid(
                "captured frontend pin differs from the admitted package root",
            ));
        }
    } else {
        match destination.symlink_metadata("repo/node_modules/@gripsack/core") {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
            Ok(_) => {
                return Err(invalid(
                    "captured frontend pin appeared without an admitted package root; capture again",
                ));
            }
        }
    }
    Ok(inventory)
}

/// Command-owned copies require read-only sealing, not durable publication.
/// A crash discards this whole capture; approvals persist only its inventory.
pub(super) fn freeze(directory: &Dir) -> io::Result<()> {
    for child in directory.read_dir(".")? {
        let name = child?.file_name();
        let metadata = directory.symlink_metadata(Path::new(&name))?;
        if metadata.is_dir() {
            let child = gripsack_fs::open_dir_nofollow(directory, Path::new(&name))?;
            freeze(&child)?;
        } else if metadata.is_file() {
            let mode = metadata.mode() & 0o7777;
            if mode != 0o400 && mode != 0o500 {
                return Err(invalid("captured source file was not sealed read-only"));
            }
        } else if !metadata.file_type().is_symlink() {
            return Err(invalid("captured source contains a special file"));
        }
    }
    directory.set_permissions(".", cap_std::fs::Permissions::from_mode(0o500))
}

pub(super) fn make_writable(directory: &Dir) -> io::Result<()> {
    directory.set_permissions(".", cap_std::fs::Permissions::from_mode(0o700))?;
    for child in directory.read_dir(".")? {
        let name = child?.file_name();
        if directory.symlink_metadata(Path::new(&name))?.is_dir() {
            let child = gripsack_fs::open_dir_nofollow(directory, Path::new(&name))?;
            make_writable(&child)?;
        }
    }
    Ok(())
}
