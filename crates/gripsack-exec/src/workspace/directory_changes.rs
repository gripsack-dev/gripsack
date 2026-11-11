//! Current declarations, not the first migration's leaf list, authorize leaf
//! operations. Directory births and retirements have their own journal domain,
//! run as the apply machine's directory sub-protocol (`apply::directories`).
use super::NativeProfiles;
use crate::ctx::ExecError;
use gripsack_host::store;
use std::{collections::BTreeMap, path::Path};

impl NativeProfiles {
    /// Every leaf the run's directory receipts must hold: each destination
    /// this run deploys and, for a subset apply, the leaves the unselected
    /// profiles carry forward.
    pub(crate) fn directory_destinations(
        &self,
        journal: &store::journal::JournalRun<'_>,
        previous: &BTreeMap<String, store::DeploymentState>,
    ) -> Result<Vec<std::path::PathBuf>, ExecError> {
        let mut destinations = self
            .profiles
            .values()
            .flat_map(|profile| &profile.files)
            .map(|file| store::journal::directory::canonical_destination(journal, &file.entry.to))
            .collect::<Result<Vec<_>, _>>()?;
        if self.partial {
            for (owner, state) in previous {
                if !self.profiles.contains_key(owner) {
                    destinations.extend(state.entries.iter().map(store::DeployedEntry::key));
                }
            }
        }
        Ok(destinations)
    }
}

pub(super) fn preview_observation(
    profiles: &NativeProfiles,
    previous: Option<&store::Generation>,
    destination: &Path,
    home: &Path,
) -> Result<Option<crate::deploy::Observation>, ExecError> {
    if let Some(previous) = previous {
        for receipt in &previous.directory_transitions {
            if destination == receipt.destination || !destination.starts_with(&receipt.destination)
            {
                continue;
            }
            let relative = destination
                .strip_prefix(&receipt.destination)
                .map_err(|_| failure("directory replacement escaped its receipt"))?;
            if receipt.directories.contains_key(relative) {
                admit_directory_replacement(profiles, previous, receipt, destination, home)?;
                // The owned shape transition retires this directory before
                // the ordinary leaf write. This is prospective, not observed absence.
                return Ok(None);
            }
            match store::journal::directory::inspect_parent(receipt, destination)? {
                store::journal::directory::ParentInspection::Existing(parent, name) => {
                    return Ok(crate::deploy::observe(&parent, &name)?);
                }
                store::journal::directory::ParentInspection::Missing => return Ok(None),
                store::journal::directory::ParentInspection::Blocked(blocker) => {
                    let cap = gripsack_host::fs::open(home)?;
                    for (owner, state) in &previous.deployments {
                        if let Some(entry) =
                            state.entries.iter().find(|entry| entry.key() == blocker)
                        {
                            if entry.preserved_drift {
                                return Err(failure(
                                    "preserved ancestor blocks directory preparation",
                                ));
                            }
                            if let Some(operation) =
                                crate::ops::plan_remove_op(owner, entry, &state.store_path, &cap)?
                                && matches!(operation.kind(), crate::ops::OpKind::Remove(_))
                                && matches!(operation.intended(), store::journal::Intended::Removed)
                            {
                                return Ok(None);
                            }
                        }
                    }
                    return Err(failure(
                        "foreign or protected ancestor blocks directory preparation",
                    ));
                }
            }
        }
    }
    Ok(crate::deploy::observe_readonly(destination)?)
}

fn admit_directory_replacement(
    profiles: &NativeProfiles,
    previous: &store::Generation,
    receipt: &store::journal::directory::DirectoryTransition,
    destination: &Path,
    home: &Path,
) -> Result<(), ExecError> {
    let mut entries = BTreeMap::new();
    for (owner, state) in &previous.deployments {
        for entry in &state.entries {
            let key = entry.key();
            if !key.starts_with(destination) {
                continue;
            }
            if profiles.partial && !profiles.profiles.contains_key(owner) {
                return Err(failure(
                    "an unselected profile retains a directory descendant",
                ));
            }
            entries.insert(key, (owner, entry, state.store_path.as_path()));
        }
    }
    let scope = destination
        .strip_prefix(&receipt.destination)
        .map_err(|_| failure("directory replacement escaped its receipt"))?;
    let home = gripsack_host::fs::open(home)?;
    for (relative, identity) in &receipt.directories {
        if !relative.starts_with(scope) {
            continue;
        }
        let (parent, name) = receipt.leaf_parent(&receipt.destination.join(relative))?;
        identity.verify(&parent, &name)?;
        let directory = gripsack_host::fs::open_dir_nofollow(&parent, &name)?;
        for child in directory.read_dir(".")? {
            let child = relative.join(child?.file_name());
            if receipt.directories.contains_key(&child) {
                continue;
            }
            let path = receipt.destination.join(&child);
            let Some((owner, entry, source)) = entries.get(&path) else {
                return Err(failure(
                    "a foreign or borrowed child prevents directory replacement",
                ));
            };
            if entry.preserved_drift {
                return Err(failure("a preserved child prevents directory replacement"));
            }
            let operation = crate::ops::plan_remove_op(owner, entry, source, &home)?;
            if !operation.is_some_and(|operation| {
                matches!(operation.kind(), crate::ops::OpKind::Remove(_))
                    && matches!(operation.intended(), store::journal::Intended::Removed)
            }) {
                return Err(failure(
                    "a drifted or protected child prevents directory replacement",
                ));
            }
        }
    }
    Ok(())
}

fn failure(message: &str) -> ExecError {
    ExecError::Step {
        output: "*".into(),
        step: "directory".into(),
        detail: message.into(),
    }
}
