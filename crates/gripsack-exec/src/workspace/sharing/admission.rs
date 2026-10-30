//! Operation-local reuse of fully admitted, unchanged retained inputs.
//!
//! A producer identity alone is never a cache key. Each decision owns the full
//! production admission and an inode/change-time snapshot of every retained
//! original, receipt and realization. Store objects are immutable; observing a
//! mutation, missing input or policy change terminates this scope. A new managed
//! operation must construct a new scope and perform production admission again.
use super::{
    format::{self, PortableManifest},
    receipt,
    runtime::{self, LoadedImport, failure},
    trust::TrustPolicy,
};
use crate::{ExecError, workspace::artifact};
use gripsack_fetch::FetchLimits;
use gripsack_fs::{
    Dir,
    cap_std::fs::{Metadata, MetadataExt},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(in crate::workspace) struct ReadmissionScope {
    home: PathBuf,
    limits: FetchLimits,
    revision: Option<String>,
    admitted: BTreeMap<PathBuf, AdmittedRoot>,
    // One inode pin per exact retained root across receipt views and admitted
    // imports. Arc clones share evidence, not additional directory handles.
    snapshots: BTreeMap<PathBuf, Arc<Snapshot>>,
    failed: bool,
    #[cfg(test)]
    loads: usize,
}

struct AdmittedRoot {
    // Keeping the production result also keeps its complete graph authority and
    // retention evidence alive. No caller can manufacture this state.
    _import: LoadedImport,
    inputs: BTreeMap<PathBuf, Arc<Snapshot>>,
}

#[derive(PartialEq, Eq)]
struct Stamp {
    device: u64,
    inode: u64,
    mode: u32,
    links: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Stamp {
    fn of(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            links: metadata.nlink(),
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

pub(in crate::workspace) struct Snapshot {
    // Hold the original root inode, preventing inode reuse while scoped.
    directory: Dir,
    home: PathBuf,
    root: PathBuf,
    entries: BTreeMap<PathBuf, Stamp>,
}
impl Snapshot {
    pub(super) fn capture(
        home: &Path,
        root: &Path,
        limits: FetchLimits,
    ) -> Result<Self, ExecError> {
        let directory = artifact::retained_directory(home, root)?
            .ok_or_else(|| failure("retained admission input missing"))?;
        let mut entries =
            BTreeMap::from([(PathBuf::from("."), Stamp::of(&directory.metadata(".")?))]);
        let mut pending = vec![PathBuf::new()];
        // Original records accompany, rather than consume, the payload-entry
        // budget. Production load remains the authority for all exact budgets.
        let bound = limits
            .archive_entries
            .get()
            .saturating_add(4 * super::closure::MAX_CLOSURE_MANIFESTS);
        while let Some(relative) = pending.pop() {
            // Queue names, not opened directories: wide valid trees must not
            // hold one descriptor per sibling while waiting to visit them.
            let parent = if relative.as_os_str().is_empty() {
                directory.try_clone()?
            } else {
                gripsack_fs::open_dir_nofollow(&directory, &relative)?
            };
            for entry in parent.entries()? {
                let name = entry?.file_name();
                let metadata = parent.symlink_metadata(&name)?;
                let path = relative.join(&name);
                if entries.len() >= bound {
                    return Err(failure("retained admission snapshot exceeds entry budget"));
                }
                if metadata.is_dir() {
                    pending.push(path.clone());
                }
                entries.insert(path, Stamp::of(&metadata));
            }
        }
        let snapshot = Self {
            directory,
            home: home.to_path_buf(),
            root: root.to_path_buf(),
            entries,
        };
        snapshot.check(home, root)?;
        Ok(snapshot)
    }

    pub(in crate::workspace) fn check(&self, home: &Path, root: &Path) -> Result<(), ExecError> {
        if home != self.home || root != self.root {
            return Err(failure(
                "retained admission snapshot belongs to another home or root",
            ));
        }
        let current = artifact::retained_directory(home, root)?
            .ok_or_else(|| failure("retained admission input missing"))?;
        if Stamp::of(&current.metadata(".")?) != self.entries[Path::new(".")] {
            return Err(failure("retained admission root changed during operation"));
        }
        // Directory change times detect added/removed names; each file's inode,
        // size, mode and change time detect rewrites, including same-size edits
        // with a restored mtime. Never follow payload symlinks or hash payloads
        // again merely because another alias uses this same admitted view.
        for (path, expected) in &self.entries {
            if Stamp::of(&self.directory.symlink_metadata(path)?) != *expected {
                return Err(failure("retained admission input changed during operation"));
            }
        }
        Ok(())
    }
}

impl ReadmissionScope {
    pub(in crate::workspace) fn new(home: &Path, limits: FetchLimits) -> Self {
        Self {
            home: home.to_path_buf(),
            limits,
            revision: None,
            admitted: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            failed: false,
            #[cfg(test)]
            loads: 0,
        }
    }

    pub(in crate::workspace) fn home(&self) -> &Path {
        &self.home
    }

    pub(in crate::workspace) fn limits(&self) -> FetchLimits {
        self.limits
    }

    pub(in crate::workspace) fn pin_input(
        &mut self,
        root: &Path,
    ) -> Result<Arc<Snapshot>, ExecError> {
        if self.failed {
            return Err(failure(
                "readmission scope previously failed; start a new operation",
            ));
        }
        let result = if let Some(snapshot) = self.snapshots.get(root) {
            snapshot
                .check(&self.home, root)
                .map(|()| Arc::clone(snapshot))
        } else {
            Snapshot::capture(&self.home, root, self.limits).map(|snapshot| {
                let snapshot = Arc::new(snapshot);
                self.snapshots
                    .insert(root.to_path_buf(), Arc::clone(&snapshot));
                snapshot
            })
        };
        if result.is_err() {
            self.admitted.clear();
            self.snapshots.clear();
            self.failed = true;
        }
        result
    }

    pub(in crate::workspace) fn check_input(&mut self, root: &Path) -> Result<(), ExecError> {
        if self.failed {
            return Err(failure(
                "readmission scope previously failed; start a new operation",
            ));
        }
        let result = self
            .snapshots
            .get(root)
            .ok_or_else(|| failure("retained input has no operation-scoped snapshot"))
            .and_then(|snapshot| snapshot.check(&self.home, root));
        if result.is_err() {
            self.admitted.clear();
            self.snapshots.clear();
            self.failed = true;
        }
        result
    }

    pub(super) fn admit_environment(
        &mut self,
        manifest: &str,
    ) -> Result<Option<Arc<super::environment::AdmittedEnvironment>>, ExecError> {
        let root = runtime::imported_root(&self.home, manifest)?;
        self.readmit_roots([root.as_path()])?;
        Ok(self.admitted[&root]._import.source.environment.clone())
    }

    pub(in crate::workspace) fn readmit_fixed<'a>(
        &mut self,
        payload: &Path,
        prefix: &Path,
        paths: impl IntoIterator<Item = &'a Path>,
    ) -> Result<(), ExecError> {
        if payload != prefix {
            self.admitted.clear();
            self.snapshots.clear();
            self.failed = true;
            return Err(failure(
                "fixed package prefix differs from its actual payload",
            ));
        }
        let paths: Vec<_> = paths.into_iter().collect();
        self.readmit_roots(paths.iter().copied())?;
        for path in paths {
            let Some(admitted) = self.admitted.get(path) else {
                continue;
            };
            let loaded = &admitted._import;
            if (loaded.facts.layout == "fixed" && loaded.facts.payload == payload)
                || loaded
                    .source
                    .environment
                    .as_ref()
                    .is_some_and(|environment| environment.admits_fixed(payload))
            {
                return Ok(());
            }
        }
        self.admitted.clear();
        self.snapshots.clear();
        self.failed = true;
        Err(failure(
            "fixed prefix has no matching signed imported placement",
        ))
    }

    pub(in crate::workspace) fn readmit_roots<'a>(
        &mut self,
        paths: impl IntoIterator<Item = &'a Path>,
    ) -> Result<(), ExecError> {
        if self.failed {
            return Err(failure(
                "readmission scope previously failed; start a new operation",
            ));
        }
        let paths: Vec<_> = paths.into_iter().collect();
        let result = self.admit(&paths);
        if result.is_err() {
            self.admitted.clear();
            self.snapshots.clear();
            self.failed = true;
        }
        result
    }

    fn check_policy(&mut self) -> Result<(), ExecError> {
        let revision = TrustPolicy::current_revision(&self.home)
            .map_err(|error| failure(error.to_string()))?;
        match &self.revision {
            Some(previous) if previous != &revision => {
                Err(failure("local sharing policy changed during operation"))
            }
            Some(_) => Ok(()),
            None => {
                self.revision = Some(revision);
                Ok(())
            }
        }
    }

    fn load(&mut self, identity: &str) -> Result<LoadedImport, ExecError> {
        #[cfg(test)]
        {
            self.loads += 1;
        }
        runtime::load(&self.home, identity, self.limits)?
            .ok_or_else(|| failure("package is not imported; import its signed bundle first"))
    }

    fn admit(&mut self, paths: &[&Path]) -> Result<(), ExecError> {
        let mut checked_policy = false;
        for &path in paths {
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("-package-import"))
            {
                continue;
            }
            if !checked_policy {
                self.check_policy()?;
                checked_policy = true;
            }
            if let Some(admitted) = self.admitted.get(path) {
                for (root, snapshot) in &admitted.inputs {
                    snapshot.check(&self.home, root)?;
                }
                continue;
            }
            let mut inputs = BTreeMap::new();
            for &root in paths {
                if !inputs.contains_key(root) {
                    inputs.insert(root.to_path_buf(), self.pin_input(root)?);
                }
            }
            let directory = artifact::retained_directory(&self.home, path)?
                .ok_or_else(|| failure("retained import receipt missing"))?;
            let bytes = receipt::read(
                &directory,
                Path::new("manifest.json"),
                format::MAX_MANIFEST_BYTES as u64,
            )?;
            let identity = PortableManifest::decode(&bytes)?.digest()?;
            if runtime::imported_root(&self.home, &identity)? != path {
                return Err(failure("import root is not bound to its manifest"));
            }
            let mut loaded = self.load(&identity)?;
            let mut incomplete = false;
            for root in loaded.retention() {
                if !inputs.contains_key(root) {
                    inputs.insert(root.clone(), self.pin_input(root)?);
                    incomplete = true;
                }
            }
            if incomplete {
                // Root-only callers did not provide a complete retained view.
                // Discover it through production admission, then admit once more
                // inside the complete snapshot; never cache post-hoc evidence.
                loaded = self.load(&identity)?;
            }
            for root in loaded.retention() {
                inputs
                    .get(root)
                    .ok_or_else(|| failure("retained admission view changed during operation"))?
                    .check(&self.home, root)?;
            }
            self.check_policy()?;
            if Some(loaded.facts.trust_revision.as_str()) != self.revision.as_deref() {
                return Err(failure("local sharing policy changed during admission"));
            }
            inputs.retain(|root, _| loaded.retention().contains(root));
            self.admitted.insert(
                path.to_path_buf(),
                AdmittedRoot {
                    _import: loaded,
                    inputs,
                },
            );
        }
        if checked_policy {
            self.check_policy()?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn production_loads(&self) -> usize {
        self.loads
    }

    #[cfg(test)]
    pub(in crate::workspace) fn pinned_inputs(&self) -> usize {
        self.snapshots.len()
    }
}
