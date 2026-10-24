//! Explicit, generation-bound transitions from retained owned links. Directory
//! transitions keep logical child paths separate from predecessor payload paths.
use crate::ExecError;
use gripsack_ir::Ownership;
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

pub struct Migration {
    entries: BTreeMap<PathBuf, MigrationEntry>,
    directories: BTreeMap<PathBuf, DirectoryMigration>,
    logical_roots: BTreeMap<PathBuf, PathBuf>,
    receipts: Vec<store::journal::directory::DirectoryTransition>,
}

struct DirectoryMigration {
    target: PathBuf,
    generation: store::GenerationId,
    leaves: Vec<PathBuf>,
}

pub struct MigrationEntry {
    target: PathBuf,
    destination: PathBuf,
}

impl Migration {
    pub fn admit<'a>(
        home: &Path,
        expected: store::GenerationId,
        current: Option<store::GenerationId>,
        previous: &store::Generation,
        destinations: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, ExecError> {
        if current != Some(expected) || previous.number != expected {
            return Err(failure(format!(
                "--migrate-from {expected} requires that exact current generation; current is {current:?}"
            )));
        }
        if store::generations::admit_manifest(home, expected)? != *previous {
            return Err(failure("migration predecessor changed during admission"));
        }
        let mut predecessors = BTreeMap::new();
        let mut declared = BTreeMap::new();
        let mut retained_roots = BTreeSet::new();
        for state in previous.modules.values() {
            retained_roots.insert(state.store_path.as_path());
            for old in &state.entries {
                declared.insert(logical(&old.to)?, old.key());
                predecessors.entry(old.key()).or_insert((state, old));
            }
        }
        let mut entries = BTreeMap::new();
        let mut directories: BTreeMap<PathBuf, DirectoryMigration> = BTreeMap::new();
        let mut logical_roots = BTreeMap::new();
        let mut verified_stores = BTreeSet::new();
        for destination in destinations {
            let expanded = logical(destination)?;
            let mut ancestor = expanded
                .ancestors()
                .skip(1)
                .find_map(|path| declared.get(path).map(|key| (path, key.clone())));
            if ancestor.is_none() {
                for candidate in expanded.ancestors().skip(1) {
                    let key = store::canonical_dest(
                        candidate
                            .to_str()
                            .ok_or_else(|| failure("non-UTF-8 destination ancestor"))?,
                    )?;
                    if predecessors.contains_key(&key) {
                        ancestor = Some((candidate, key));
                        break;
                    }
                }
            }
            if let Some((declared_parent, key)) = ancestor {
                if key.ancestors().any(|path| retained_roots.contains(path)) {
                    return Err(failure(
                        "directory predecessor resolves inside retained store content",
                    ));
                }
                let (state, old) = predecessors[&key];
                let target = authorize(state, old, &key, &mut verified_stores)?;
                if !std::fs::symlink_metadata(&target)?.is_dir() {
                    return Err(failure(format!(
                        "{destination}: predecessor is not a real directory payload"
                    )));
                }
                let relative = expanded
                    .strip_prefix(declared_parent)
                    .map_err(|_| failure("logical child escaped parent"))?
                    .to_owned();
                logical_roots.insert(declared_parent.to_owned(), key.clone());
                directories
                    .entry(key)
                    .or_insert_with(|| DirectoryMigration {
                        target,
                        generation: expected,
                        leaves: Vec::new(),
                    })
                    .leaves
                    .push(relative);
                continue;
            }
            let key = store::canonical_dest(destination)?;
            if key.ancestors().any(|path| retained_roots.contains(path)) {
                return Err(failure(format!(
                    "{destination}: destination resolves into retained content"
                )));
            }
            let Some(&(state, old)) = predecessors.get(&key) else {
                match std::fs::symlink_metadata(&key) {
                    Ok(_) => {
                        return Err(failure(format!(
                            "{destination}: foreign destination is not authorized by --migrate-from"
                        )));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                continue;
            };
            if old.preserved_drift {
                return Err(failure(format!(
                    "{destination}: predecessor records preserved drift"
                )));
            }
            if !matches!(
                old.ownership,
                store::StoredOwnership::Legacy(Ownership::Owned)
            ) {
                continue;
            }
            let target = authorize(state, old, &key, &mut verified_stores)?;
            entries.insert(
                key.clone(),
                MigrationEntry {
                    destination: key,
                    target,
                },
            );
        }
        for root in directories.keys() {
            if root
                .ancestors()
                .skip(1)
                .any(|ancestor| directories.contains_key(ancestor))
            {
                return Err(failure("directory transition roots overlap"));
            }
        }
        for (root, directory) in &mut directories {
            directory.leaves.sort();
            directory.leaves.dedup();
            let leaves: BTreeSet<&Path> = directory.leaves.iter().map(PathBuf::as_path).collect();
            if leaves.iter().any(|leaf| {
                leaf.ancestors()
                    .skip(1)
                    .any(|ancestor| leaves.contains(ancestor))
            }) || entries.contains_key(root)
            {
                return Err(failure(
                    "directory transition has colliding parent/child declarations",
                ));
            }
        }
        if entries.is_empty() && directories.is_empty() {
            return Err(failure(
                "--migrate-from selects no overlapping legacy-owned destinations",
            ));
        }
        Ok(Self {
            entries,
            directories,
            logical_roots,
            receipts: Vec::new(),
        })
    }

    pub fn entry(&self, destination: &Path) -> Option<&MigrationEntry> {
        self.entries.get(destination)
    }

    pub(crate) fn directory_destinations(&self) -> impl Iterator<Item = &PathBuf> {
        self.directories.keys()
    }

    pub(crate) fn owns_child(&self, destination: &Path) -> bool {
        self.directories.iter().any(|(root, transition)| {
            destination
                .strip_prefix(root)
                .is_ok_and(|relative| transition.leaves.iter().any(|leaf| leaf == relative))
        })
    }

    pub fn destination(&self, destination: &str) -> Result<PathBuf, std::io::Error> {
        let expanded = logical(destination)?;
        for (declared, root) in &self.logical_roots {
            if let Ok(relative) = expanded.strip_prefix(declared)
                && !relative.as_os_str().is_empty()
            {
                return Ok(root.join(relative));
            }
        }
        store::canonical_dest(destination)
    }

    /// Read-only uniqueness projection before artifacts are prepared. This
    /// grants no write authority; admit() still checks every exact predecessor.
    pub fn destination_for_predecessor(
        previous: &store::Generation,
        destination: &str,
    ) -> std::io::Result<PathBuf> {
        let expanded = logical(destination)?;
        for state in previous.modules.values() {
            for entry in &state.entries {
                if matches!(
                    entry.ownership,
                    store::StoredOwnership::Legacy(Ownership::Owned)
                ) && let Ok(relative) = expanded.strip_prefix(logical(&entry.to)?)
                    && !relative.as_os_str().is_empty()
                {
                    return Ok(entry.key().join(relative));
                }
            }
        }
        for ancestor in expanded.ancestors().skip(1) {
            let key = store::canonical_dest(ancestor.to_str().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "non-UTF-8 destination ancestor",
                )
            })?)?;
            if previous
                .modules
                .values()
                .flat_map(|state| &state.entries)
                .any(|entry| {
                    matches!(
                        entry.ownership,
                        store::StoredOwnership::Legacy(Ownership::Owned)
                    ) && entry.key() == key
                })
            {
                return Ok(key.join(expanded.strip_prefix(ancestor).expect("ancestor prefix")));
            }
        }
        store::canonical_dest(destination)
    }

    pub fn publish_directories(
        &mut self,
        run: &store::journal::JournalRun<'_>,
    ) -> Result<(), ExecError> {
        for (destination, planned) in &self.directories {
            let receipt = store::journal::directory::prepare(
                destination.clone(),
                planned.target.clone(),
                planned.generation,
                planned.leaves.clone(),
            )?;
            store::journal::directory::publish(run, &receipt)?;
            self.receipts.push(receipt);
        }
        Ok(())
    }

    pub fn receipts(&self) -> &[store::journal::directory::DirectoryTransition] {
        &self.receipts
    }

    pub(crate) fn release_entries(&self, modules: &mut BTreeMap<String, store::ModuleState>) {
        for state in modules.values_mut() {
            state.entries.retain(|entry| {
                !self.entries.contains_key(&entry.key())
                    && !self.directories.contains_key(&entry.key())
            });
        }
    }
}

fn authorize<'a>(
    state: &'a store::ModuleState,
    old: &store::DeployedEntry,
    key: &Path,
    verified_stores: &mut BTreeSet<&'a Path>,
) -> Result<PathBuf, ExecError> {
    if old.preserved_drift
        || !matches!(
            old.ownership,
            store::StoredOwnership::Legacy(Ownership::Owned)
        )
    {
        return Err(failure(
            "directory migration requires exact retained owned-link authority",
        ));
    }
    if let Some(expected) = &state.tree256
        && verified_stores.insert(state.store_path.as_path())
        && store::canonical_tree_hash(&state.store_path)?.as_str() != expected.as_str()
    {
        return Err(failure("predecessor immutable payload tree drifted"));
    }
    let target = state.store_path.join(&old.from);
    if std::fs::read_link(key)? != target {
        return Err(failure(
            "predecessor link drifted; migration refuses to absorb it",
        ));
    }
    if store::canonical_file_hash(&target)?.as_str() != old.hash.as_str() {
        return Err(failure("predecessor payload identity drifted"));
    }
    Ok(target)
}

fn logical(destination: &str) -> std::io::Result<PathBuf> {
    let expanded = store::expand_home(destination);
    if !expanded.is_absolute()
        || expanded.to_str().is_none()
        || expanded
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "migration destination must be an absolute normalized UTF-8 path",
        ));
    }
    Ok(expanded.components().collect())
}

impl MigrationEntry {
    pub(crate) fn reobserve(&self, view: &crate::ops::DestView<'_>) -> Result<(), ExecError> {
        if view.dest != self.destination {
            return Err(failure(
                "migration authorization belongs to a different destination",
            ));
        }
        match &view.observed {
            Some(super::Observation::Symlink { target }) if Path::new(target) == self.target => {
                Ok(())
            }
            _ => Err(failure(format!(
                "{}: predecessor changed after migration admission",
                view.entry.to
            ))),
        }
    }
}

fn failure(detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        module: "*".into(),
        step: "migrate".into(),
        detail: detail.into(),
    }
}
