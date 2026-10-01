//! Command-owned captured source. Only copied, inventoried, read-only objects
//! are exposed to the evaluator; approval never authenticates a live worktree.
mod capture;
mod inventory;
mod materialization;
mod resolve;
#[cfg(test)]
mod tests;

use gripsack_fs::Dir;
use inventory::{
    MAX_ENTRIES, MAX_INVENTORY_BYTES, MAX_RESOLUTION_STEPS, MAX_SOURCE_BYTES, invalid,
};
pub use inventory::{
    SourceBundleDigest, SourceEntry, SourceFileBytes, SourceInventory, SourceObject, SourceRootKind,
};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

pub(super) struct CaptureRoot {
    pub kind: SourceRootKind,
    pub canonical: PathBuf,
    pub declared: PathBuf,
    pub directory: Dir,
}

impl CaptureRoot {
    fn open(kind: SourceRootKind, path: &Path) -> io::Result<Self> {
        let declared = std::path::absolute(path)?;
        let canonical = path.canonicalize()?;
        let directory = gripsack_fs::open(&canonical)?;
        Ok(Self {
            kind,
            canonical,
            declared,
            directory,
        })
    }

    pub fn logical(&self, relative: &Path) -> PathBuf {
        let mut path = PathBuf::from(self.kind.directory());
        path.extend(relative.components());
        path
    }
}

#[derive(Default)]
pub(super) struct CaptureBudget {
    entries: usize,
    path_bytes: usize,
    content_bytes: u64,
    resolution_steps: usize,
}

impl CaptureBudget {
    pub fn entry(&mut self, path: &str) -> io::Result<()> {
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or_else(|| invalid("source entry count overflow"))?;
        self.metadata_bytes(path.len())?;
        if self.entries > MAX_ENTRIES {
            return Err(invalid("source capture exceeds its entry/path budget"));
        }
        Ok(())
    }

    pub fn metadata_bytes(&mut self, bytes: usize) -> io::Result<()> {
        self.path_bytes = self
            .path_bytes
            .checked_add(bytes)
            .ok_or_else(|| invalid("source metadata size overflow"))?;
        if self.path_bytes > MAX_INVENTORY_BYTES {
            return Err(invalid("source capture exceeds its metadata budget"));
        }
        Ok(())
    }

    pub fn content(&mut self, bytes: usize) -> io::Result<()> {
        let bytes =
            u64::try_from(bytes).map_err(|_| invalid("source byte count exceeds its domain"))?;
        self.content_bytes = self
            .content_bytes
            .checked_add(bytes)
            .ok_or_else(|| invalid("source byte count overflow"))?;
        if self.content_bytes > MAX_SOURCE_BYTES {
            return Err(invalid("source capture exceeds its total byte budget"));
        }
        Ok(())
    }

    pub fn resolve_step(&mut self) -> io::Result<()> {
        self.resolution_steps = self
            .resolution_steps
            .checked_add(1)
            .ok_or_else(|| invalid("source traversal overflow"))?;
        if self.resolution_steps > MAX_RESOLUTION_STEPS {
            return Err(invalid("source capture exceeds its resolution budget"));
        }
        Ok(())
    }
}

/// Private staging remains owned even when sealing fails partway through. Drop
/// restores only directory owner access, never follows a captured alias, then
/// lets TempDir remove this command's own files.
#[derive(Debug)]
struct OwnedDirectory {
    directory: Dir,
    temporary: tempfile::TempDir,
}

impl Drop for OwnedDirectory {
    fn drop(&mut self) {
        let _ = capture::make_writable(&self.directory);
    }
}

#[derive(Debug)]
pub struct SourceBundle {
    _owned: OwnedDirectory,
    root: Dir,
    repository_identity: PathBuf,
    frontend_origin: PathBuf,
    pinned_origin: Option<PathBuf>,
    repository: PathBuf,
    frontend: PathBuf,
    pinned_frontend: Option<PathBuf>,
    inventory: SourceInventory,
    inventory_bytes: Vec<u8>,
    digest: SourceBundleDigest,
}

impl SourceBundle {
    /// `pin` is the repo-selected @gripsack/core root, if present. Its name is
    /// admitted through the pinned source capability before copying that root,
    /// and checked again on the copied package before returning authority.
    pub fn capture(
        repo: &Path,
        frontend: &Path,
        pin: Option<&Path>,
        runtime_home: &Path,
    ) -> io::Result<Self> {
        let mut roots = vec![
            CaptureRoot::open(SourceRootKind::Repository, repo)?,
            CaptureRoot::open(SourceRootKind::Frontend, frontend)?,
        ];
        if let Some(pin) = pin {
            let root = CaptureRoot::open(SourceRootKind::PinnedFrontend, pin)?;
            admit_pin(&root.directory)?;
            roots.push(root);
        }
        let temporary = tempfile::Builder::new()
            .prefix("gripsack-source-")
            .tempdir()?;
        let owned = OwnedDirectory {
            directory: gripsack_fs::open(temporary.path())?,
            temporary,
        };
        if roots
            .iter()
            .any(|root| owned.temporary.path().starts_with(&root.canonical))
        {
            return Err(invalid("source root contains its own capture directory"));
        }
        owned.directory.create_dir("stage")?;
        let stage = gripsack_fs::open_dir_nofollow(&owned.directory, Path::new("stage"))?;
        let runtime_home = match runtime_home.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                std::path::absolute(runtime_home)?
            }
            Err(error) => return Err(error),
        };
        let mut budget = CaptureBudget::default();
        let inventory = capture::copy_roots(&roots, &stage, &runtime_home, &mut budget)?;
        if pin.is_some() {
            let pinned = gripsack_fs::open_dir_nofollow(&stage, Path::new("pin"))?;
            admit_pin(&pinned)?;
        }
        let inventory_bytes = inventory.encode()?;
        let digest = SourceBundleDigest::of_inventory(&inventory_bytes);
        // Finalize the private directory's name while it remains writable.
        // No captured authority escapes until read-only sealing succeeds.
        let name = digest.to_string();
        owned.directory.rename("stage", &owned.directory, &name)?;
        capture::freeze(&stage)?;
        let root = owned.temporary.path().join(name);
        let repository = root.join("repo");
        let frontend = root.join("frontend");
        let pinned_frontend = pin.map(|_| root.join("pin"));
        let mut roots = roots.into_iter();
        let repository_identity = roots.next().expect("repository root is required").canonical;
        let frontend_origin = roots.next().expect("frontend root is required").canonical;
        let pinned_origin = roots.next().map(|root| root.canonical);
        Ok(Self {
            _owned: owned,
            root: stage,
            repository_identity,
            frontend_origin,
            pinned_origin,
            repository,
            frontend,
            pinned_frontend,
            inventory,
            inventory_bytes,
            digest,
        })
    }

    pub fn repository_identity(&self) -> &Path {
        &self.repository_identity
    }
    pub fn repository(&self) -> &Path {
        &self.repository
    }
    pub fn frontend(&self) -> &Path {
        &self.frontend
    }
    pub fn pinned_frontend(&self) -> Option<&Path> {
        self.pinned_frontend.as_deref()
    }
    pub fn inventory(&self) -> &SourceInventory {
        &self.inventory
    }
    pub fn inventory_bytes(&self) -> &[u8] {
        &self.inventory_bytes
    }
    pub fn digest(&self) -> SourceBundleDigest {
        self.digest
    }

    pub fn original_root(&self, kind: SourceRootKind) -> Option<&Path> {
        match kind {
            SourceRootKind::Repository => Some(&self.repository_identity),
            SourceRootKind::Frontend => Some(&self.frontend_origin),
            SourceRootKind::PinnedFrontend => self.pinned_origin.as_deref(),
        }
    }

    pub fn captured_root(&self, kind: SourceRootKind) -> Option<&Path> {
        match kind {
            SourceRootKind::Repository => Some(&self.repository),
            SourceRootKind::Frontend => Some(&self.frontend),
            SourceRootKind::PinnedFrontend => self.pinned_frontend.as_deref(),
        }
    }

    /// Bind explicitly declared native paths inside an admitted source root to
    /// that root's captured object. Bare names remain operator-PATH selections;
    /// explicit outside paths retain their declared native-action meaning.
    pub fn native_path<'a>(&self, value: &'a str) -> io::Result<std::borrow::Cow<'a, str>> {
        use std::{borrow::Cow, path::Component};
        let path = Path::new(value);
        if path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(invalid(
                "native plugin paths cannot contain parent traversal; use an explicit absolute path",
            ));
        }
        if !path.is_absolute() {
            if path.components().count() == 1
                && matches!(path.components().next(), Some(Component::Normal(_)))
            {
                return Ok(Cow::Borrowed(value));
            }
            return self
                .repository
                .join(path)
                .into_os_string()
                .into_string()
                .map(Cow::Owned)
                .map_err(|_| invalid("captured native path is not UTF-8"));
        }
        for &kind in self.inventory.roots().iter().rev() {
            if let (Some(original), Some(captured)) =
                (self.original_root(kind), self.captured_root(kind))
                && let Ok(relative) = path.strip_prefix(original)
            {
                return captured
                    .join(relative)
                    .into_os_string()
                    .into_string()
                    .map(Cow::Owned)
                    .map_err(|_| invalid("captured native path is not UTF-8"));
            }
        }
        Ok(Cow::Borrowed(value))
    }

    /// Diagnostics use logical source coordinates. Relative labels need no
    /// allocation; absolute native/file-URL labels are rebound only inside a
    /// captured root, never by guessing an original path from a basename.
    pub fn logical_text<'a>(&self, text: &'a str) -> std::borrow::Cow<'a, str> {
        use std::borrow::Cow;
        let name = self
            .repository
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str());
        if name.is_none_or(|name| !text.contains(name)) {
            return Cow::Borrowed(text);
        }
        let mut result = Cow::Borrowed(text);
        for &kind in self.inventory.roots() {
            let (Some(captured), Some(original)) =
                (self.captured_root(kind), self.original_root(kind))
            else {
                continue;
            };
            if result.contains("file://")
                && let (Ok(from), Ok(to)) = (
                    url::Url::from_directory_path(captured),
                    url::Url::from_directory_path(original),
                )
                && result.contains(from.as_str())
            {
                result = Cow::Owned(result.replace(from.as_str(), to.as_str()));
            }
            if let (Some(captured), Some(original)) = (captured.to_str(), original.to_str()) {
                if result == captured {
                    result = Cow::Owned(original.to_owned());
                    continue;
                }
                let from = format!("{captured}/");
                if result.contains(&from) {
                    result = Cow::Owned(result.replace(&from, &format!("{original}/")));
                }
            }
        }
        result
    }

    /// The captured file's original mode is data for native materialization,
    /// not permission to make the evaluator's read-only copy writable.
    pub fn original_mode(&self, relative: &Path) -> io::Result<crate::prior::FileMode> {
        let canonical = self.root.canonicalize(Path::new("repo").join(relative))?;
        let path = canonical
            .to_str()
            .ok_or_else(|| invalid("source path is not UTF-8"))?;
        match self.inventory.entry(path).map(|entry| &entry.object) {
            Some(SourceObject::File { mode, .. }) => Ok(*mode),
            _ => Err(invalid(
                "captured source file has no admitted original mode",
            )),
        }
    }
}

fn admit_pin(directory: &Dir) -> io::Result<()> {
    const PACKAGE_METADATA_BYTES: u64 = 1024 * 1024;
    let mut file = gripsack_fs::open_file_nofollow(directory, Path::new("package.json"))?;
    if file.metadata()?.len() > PACKAGE_METADATA_BYTES {
        return Err(invalid(
            "pinned frontend package metadata exceeds its byte budget",
        ));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(PACKAGE_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > PACKAGE_METADATA_BYTES {
        return Err(invalid(
            "pinned frontend package metadata exceeds its byte budget",
        ));
    }
    let package: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if package.get("name").and_then(serde_json::Value::as_str) != Some("@gripsack/core") {
        return Err(invalid(
            "the pinned frontend root is not an @gripsack/core package",
        ));
    }
    Ok(())
}
