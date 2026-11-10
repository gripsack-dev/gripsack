//! The host side of the directory sub-protocol for apply (plan/0056 F2 step
//! 9, `gripsack_kernel::directory`).
//!
//! The directory domain (journal v3 receipts: migrated directories, their
//! evolution and retirement) stays host code; the apply machine decides when
//! each of its steps runs and what it may touch. Before the first deploy the
//! receipts the previous generation recorded are bound, a migration's owned
//! links are born as directories, every receipt's evolution is planned, what
//! stands where a planned node must go is cleared — as whole-object units the
//! machine decides (gripsack's own write is removed, anything else refuses
//! the run) —, the evolutions are published, and every owned node is held.
//! After the last release every receipt is finished (nodes no leaf needs any
//! more are retired) or, with no leaf left, retired to its predecessor link.
//! Nothing here touches a receipt's namespace except inside a step the
//! machine commanded ([`Machine::directory_step`]).
use super::{Machine, Purpose, remove_unit};
use crate::ctx::ExecError;
use gripsack_kernel::directory::Action;
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Everything before the first deploy: bind, birth, plan, clear, evolve,
/// hold. `receipts` are the previous generation's; `destinations` every leaf
/// this run deploys (and, for a subset apply, the leaves it carries forward).
pub(crate) fn open(
    journal: &store::journal::JournalRun<'_>,
    previous: &BTreeMap<String, store::DeploymentState>,
    receipts: &[store::journal::directory::DirectoryTransition],
    migration: Option<&crate::deploy::Migration>,
    destinations: &[PathBuf],
    machine: &mut Machine,
) -> Result<(), ExecError> {
    for receipt in receipts {
        // a journal record only: the namespace is unchanged
        machine.directory_step(&receipt.destination, Action::Bind, || {
            store::journal::directory::bind_existing(journal, receipt)?;
            Ok(false)
        })?;
    }
    if let Some(migration) = migration {
        crate::util::crash_hook("directory-before-publish");
        for destination in migration.directory_destinations() {
            machine.directory_step(destination, Action::Birth, || {
                migration.birth(journal, destination)?;
                Ok(true)
            })?;
        }
        crate::util::crash_hook("directory-after-publish");
    }
    let mut plans = Vec::new();
    for receipt in store::journal::directory::receipts(journal)? {
        let root = receipt.destination;
        let mut planned = None;
        // preparing a node's private staging name changes no destination
        machine.directory_step(&root, Action::Plan, || {
            planned = store::journal::directory::plan(journal, &root, destinations)?;
            Ok(false)
        })?;
        if let Some(plan) = planned {
            plans.push((root, plan));
        }
    }
    clear_blockers(journal, previous, &plans, machine)?;
    for (root, plan) in plans {
        crate::util::crash_hook("directory-evolve-before-publish");
        machine.directory_step(&root, Action::Evolve, || Ok(plan.publish(journal)?))?;
        crate::util::crash_hook("directory-evolve-after-publish");
    }
    for receipt in store::journal::directory::receipts(journal)? {
        for relative in receipt.directories.keys() {
            machine.hold(&receipt.destination.join(relative))?;
        }
    }
    Ok(())
}

/// What stands where a planned node must go: the previous generation's
/// entries at or below each blocker are cleared by the machine (each once);
/// a blocker no entry records and no receipt owns is foreign and refused.
/// Every blocker is admitted read-only first, so a
/// refusal of a later one never follows the removal of an earlier one.
fn clear_blockers(
    journal: &store::journal::JournalRun<'_>,
    previous: &BTreeMap<String, store::DeploymentState>,
    plans: &[(PathBuf, store::journal::directory::EvolutionPlan)],
    machine: &mut Machine,
) -> Result<(), ExecError> {
    let entries: Vec<_> = previous
        .iter()
        .flat_map(|(owner, state)| {
            state.entries.iter().map(move |entry| {
                (
                    entry.ownership_key(owner),
                    owner.as_str(),
                    entry,
                    state.store_path.as_path(),
                )
            })
        })
        .collect();
    admit_blockers(journal, &entries, plans)?;
    let mut cleared = BTreeSet::new();
    for (_, plan) in plans {
        for blocker in plan.blockers() {
            for (key, owner, entry, source) in &entries {
                if !key.destination().starts_with(blocker) {
                    continue;
                }
                if cleared.insert(key.clone()) {
                    remove_unit(owner, entry, source, journal, machine, Purpose::Clear)?;
                }
            }
        }
    }
    Ok(())
}

/// The read-only admission of every blocker before any is cleared, with the
/// planner's own judgement: each entry must be gripsack's unchanged write
/// with nothing to bring back, and an unrecorded blocker must be a node a
/// receipt owns. The machine decides each clear again before it acts.
fn admit_blockers(
    journal: &store::journal::JournalRun<'_>,
    entries: &[(store::OwnershipKey, &str, &store::DeployedEntry, &Path)],
    plans: &[(PathBuf, store::journal::directory::EvolutionPlan)],
) -> Result<(), ExecError> {
    let refuse = |detail: &str| ExecError::Step {
        output: "*".into(),
        step: "directory".into(),
        detail: detail.into(),
    };
    for (_, plan) in plans {
        for blocker in plan.blockers() {
            let mut recorded = false;
            for (key, owner, entry, source) in entries {
                if !key.destination().starts_with(blocker) {
                    continue;
                }
                recorded = true;
                if entry.preserved_drift {
                    return Err(refuse("preserved file blocks a new directory ancestor"));
                }
                if let Some(operation) =
                    crate::ops::plan_remove_op(owner, entry, source, journal.home())?
                {
                    if matches!(operation.kind(), crate::ops::OpKind::Preserved) {
                        return Err(refuse(
                            "foreign file blocks a new directory ancestor; no directory takeover is granted",
                        ));
                    }
                    if !matches!(operation.intended(), store::journal::Intended::Removed) {
                        return Err(refuse(
                            "retained content prevents removal of a new directory ancestor",
                        ));
                    }
                }
            }
            // an owned node being replaced by a file is the receipt's own:
            // its evolution retires it; anything else unrecorded is foreign
            if !recorded && !store::journal::directory::is_directory(journal, blocker)? {
                return Err(refuse("foreign object blocks a new directory ancestor"));
            }
        }
    }
    Ok(())
}

/// Everything after the last release: each active receipt finished for the
/// leaves the new manifest records inside it, or retired when none is left.
/// Returns the receipts the new generation records.
pub(crate) fn close(
    journal: &store::journal::JournalRun<'_>,
    deployments: &BTreeMap<String, store::DeploymentState>,
    machine: &mut Machine,
) -> Result<Vec<store::journal::directory::DirectoryTransition>, ExecError> {
    let mut transitions = Vec::new();
    for receipt in store::journal::directory::receipts(journal)? {
        let leaves: Vec<_> = deployments
            .values()
            .flat_map(|state| &state.entries)
            .filter_map(|entry| {
                entry
                    .key()
                    .strip_prefix(&receipt.destination)
                    .ok()
                    .filter(|relative| !relative.as_os_str().is_empty())
                    .map(Path::to_owned)
            })
            .collect();
        if leaves.is_empty() {
            machine.directory_step(&receipt.destination, Action::Retire, || {
                crate::deploy::directory_retirement::retire(journal, &receipt)?;
                Ok(true)
            })?;
        } else {
            machine.directory_step(&receipt.destination, Action::Finish, || {
                let finished =
                    store::journal::directory::finish(journal, &receipt.destination, leaves)?;
                let changed = finished != receipt;
                transitions.push(finished);
                Ok(changed)
            })?;
        }
    }
    Ok(transitions)
}
