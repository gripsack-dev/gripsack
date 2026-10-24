//! Current declarations, not the first migration's leaf list, authorize leaf
//! operations. Directory births and retirements have their own journal domain.
use super::NativeProfiles;
use crate::ctx::{Ctx, ExecError};
use gripsack_store as store;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

impl NativeProfiles {
    pub(crate) fn prepare_directory_changes(
        &self,
        _ctx: &Ctx,
        journal: &store::journal::JournalRun<'_>,
        previous: &BTreeMap<String, store::ModuleState>,
    ) -> Result<(), ExecError> {
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
        let plans = store::journal::directory::plans(journal, &destinations)?;
        let entries: Vec<_> = previous
            .iter()
            .flat_map(|(owner, state)| {
                state.entries.iter().map(move |entry| {
                    (
                        entry.ownership_key(owner),
                        owner,
                        entry,
                        state.store_path.as_path(),
                    )
                })
            })
            .collect();
        let mut released = BTreeSet::new();
        for plan in plans {
            for blocker in plan.blockers() {
                let mut authorized = store::journal::directory::is_directory(journal, blocker)?;
                for (key, owner, entry, source) in &entries {
                    if key.destination() != blocker && !key.destination().starts_with(blocker) {
                        continue;
                    }
                    authorized = true;
                    if !released.insert(key.clone()) {
                        continue;
                    }
                    if entry.preserved_drift {
                        return Err(failure("preserved file blocks a new directory ancestor"));
                    }
                    if let Some(operation) =
                        crate::ops::plan_remove_op(owner, entry, source, journal.home())?
                    {
                        if matches!(operation.kind(), crate::ops::OpKind::Preserved) {
                            return Err(failure(
                                "foreign file blocks a new directory ancestor; no directory takeover is granted",
                            ));
                        }
                        crate::ops::execute_op(journal, operation.as_executable()?)?;
                    }
                }
                if !authorized {
                    return Err(failure("foreign object blocks a new directory ancestor"));
                }
            }
            crate::util::crash_hook("directory-evolve-before-publish");
            plan.publish(journal)?;
            crate::util::crash_hook("directory-evolve-after-publish");
        }
        Ok(())
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
                    let cap = gripsack_fs::open(home)?;
                    for (owner, state) in &previous.modules {
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
    for (owner, state) in &previous.modules {
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
    let home = gripsack_fs::open(home)?;
    for (relative, identity) in &receipt.directories {
        if !relative.starts_with(scope) {
            continue;
        }
        let (parent, name) = receipt.leaf_parent(&receipt.destination.join(relative))?;
        identity.verify(&parent, &name)?;
        let directory = gripsack_fs::open_dir_nofollow(&parent, &name)?;
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
        module: "*".into(),
        step: "directory".into(),
        detail: message.into(),
    }
}
