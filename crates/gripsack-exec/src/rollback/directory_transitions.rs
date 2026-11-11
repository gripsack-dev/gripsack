//! The host side of the directory sub-protocol for rollback (plan/0056 F2
//! step 9, `gripsack_kernel::directory`): retained tree rollback evolves
//! shared root shapes and exchanges distinct incarnations without moving
//! foreign contents.
//!
//! The rollback machine commands every step here, in stage order: the
//! current generation's receipts are bound; the current children a shape
//! change takes away are cleared or released before it, as whole-object
//! units the machine decides ([`clearing`] names them); each target receipt's shape is restored,
//! exchanged or reactivated ([`Step`]); its nodes are held; and once every
//! unit settled, a receipt the target does not keep is retired to its
//! predecessor link.
use super::DestMap;
use crate::ExecError;
use gripsack_host::store::{
    self as store,
    journal::{ObjectIdentity, directory::DirectoryTransition},
};
use gripsack_kernel::directory::Action;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// One directory step of a rollback, for one receipt.
pub(super) enum Step<'m> {
    /// Bind a current receipt into the run's journal.
    Bind(&'m DirectoryTransition),
    /// Restore the target's shape over the same root incarnation.
    Shape(&'m DirectoryTransition),
    /// Reverse the current incarnation, then reactivate the target's.
    Exchange {
        current: &'m DirectoryTransition,
        target: &'m DirectoryTransition,
    },
    /// Reactivate the target's retained incarnation where no receipt stands.
    Reactivate(&'m DirectoryTransition),
    /// Retire a current receipt the target does not keep (reverse it to its
    /// predecessor link); the caller then settles that link.
    Retire(&'m DirectoryTransition),
}

impl Step<'_> {
    pub(super) fn root(&self) -> &Path {
        match self {
            Step::Bind(receipt)
            | Step::Shape(receipt)
            | Step::Reactivate(receipt)
            | Step::Retire(receipt) => &receipt.destination,
            Step::Exchange { target, .. } => &target.destination,
        }
    }

    pub(super) fn action(&self) -> Action {
        match self {
            Step::Bind(_) => Action::Bind,
            Step::Shape(_) => Action::Shape,
            Step::Exchange { .. } => Action::Exchange,
            Step::Reactivate(_) => Action::Reactivate,
            Step::Retire(_) => Action::Retire,
        }
    }
}

/// The current receipt at the target receipt's root, if any.
pub(super) fn previous<'m>(
    current: &'m [DirectoryTransition],
    target: &DirectoryTransition,
) -> Option<&'m DirectoryTransition> {
    current
        .iter()
        .find(|previous| previous.destination == target.destination)
}

/// Whether the target restores the shape over the current root incarnation.
fn same_root(previous: Option<&DirectoryTransition>, target: &DirectoryTransition) -> bool {
    previous.is_some_and(|previous| {
        previous.directories.get(Path::new("")) == target.directories.get(Path::new(""))
    })
}

/// The shape step a target receipt needs.
pub(super) fn shaping<'m>(
    current: &'m [DirectoryTransition],
    target: &'m DirectoryTransition,
) -> Step<'m> {
    let previous = previous(current, target);
    match previous {
        _ if same_root(previous, target) => Step::Shape(target),
        Some(current) => Step::Exchange { current, target },
        None => Step::Reactivate(target),
    }
}

/// The current children a target receipt's shape change takes away, deepest
/// first, settled before it: `clear` — every child of a replaced
/// incarnation, and below a kept root the children of a node that changes
/// and a file where the target needs a node (they must go) — and `release`
/// — the other children the target no longer deploys (the prune rule: the
/// user's change is kept). The rest are ordinary units after the shape
/// change.
pub(super) struct Clearing {
    pub clear: Vec<store::OwnershipKey>,
    pub release: Vec<store::OwnershipKey>,
}

pub(super) fn clearing(
    current: &DestMap<'_>,
    target_keys: &BTreeSet<store::OwnershipKey>,
    current_directories: &[DirectoryTransition],
    target: &DirectoryTransition,
) -> Clearing {
    let mut found = Clearing {
        clear: Vec::new(),
        release: Vec::new(),
    };
    let Some(previous) = previous(current_directories, target) else {
        return found;
    };
    let same_root = same_root(Some(previous), target);
    let changing: Vec<PathBuf> = previous
        .directories
        .iter()
        .filter(|(relative, identity)| {
            !relative.as_os_str().is_empty() && target.directories.get(*relative) != Some(*identity)
        })
        .map(|(relative, _)| target.destination.join(relative))
        .collect();
    let needed_nodes: BTreeSet<_> = target
        .directories
        .keys()
        .map(|relative| target.destination.join(relative))
        .collect();
    let mut children: Vec<_> = current
        .keys()
        .filter(|key| {
            key.destination() != target.destination
                && key.destination().starts_with(&target.destination)
        })
        .collect();
    children.sort_by_key(|key| std::cmp::Reverse(key.destination().components().count()));
    for key in children {
        let structural = !same_root
            || changing
                .iter()
                .any(|root| key.destination().starts_with(root))
            || needed_nodes.contains(key.destination());
        if structural {
            found.clear.push(key.clone());
        } else if !target_keys.contains(key) {
            found.release.push(key.clone());
        }
    }
    found
}

/// Execute one commanded step (`Retire` only reverses; the caller settles
/// the predecessor link it restores).
pub(super) fn execute(
    journal: &store::journal::JournalRun<'_>,
    current: &DestMap<'_>,
    step: &Step<'_>,
) -> Result<(), ExecError> {
    match *step {
        Step::Bind(receipt) => {
            store::journal::directory::bind_existing(journal, receipt)?;
        }
        Step::Shape(target) => {
            target.verify_borrowed()?;
            store::journal::directory::restore_shape(journal, target)?;
        }
        Step::Exchange {
            current: previous,
            target,
        } => {
            store::journal::reverse_directory(journal, previous)?;
            let link = ObjectIdentity::Link(
                previous
                    .predecessor_target
                    .to_str()
                    .ok_or_else(|| failure("non-UTF-8 predecessor"))?
                    .to_owned(),
            );
            store::journal::directory::reactivate(journal, target, Some(&link))?;
        }
        Step::Reactivate(target) => {
            let key = store::OwnershipKey::new(target.destination.clone(), None);
            let expected = match current.get(&key) {
                None => restored_origin(journal, target)?,
                Some((owner, entry, source)) => {
                    if entry.preserved_drift {
                        return Err(failure("preserved root prevents directory reactivation"));
                    }
                    match crate::ops::plan_remove_op(owner, entry, source, journal.home())? {
                        None => None,
                        Some(operation)
                            if matches!(operation.kind(), crate::ops::OpKind::Preserved) =>
                        {
                            return Err(failure("foreign root prevents directory reactivation"));
                        }
                        Some(operation) => operation.observed().cloned(),
                    }
                }
            };
            store::journal::directory::reactivate(journal, target, expected.as_ref())?;
        }
        Step::Retire(receipt) => {
            store::journal::reverse_directory(journal, receipt)?;
        }
    }
    Ok(())
}

fn restored_origin(
    journal: &store::journal::JournalRun<'_>,
    receipt: &DirectoryTransition,
) -> Result<Option<ObjectIdentity>, ExecError> {
    let predecessor =
        store::generations::admit_manifest(journal.home_path(), receipt.predecessor_generation)?;
    let (state, entry) = predecessor
        .deployments
        .values()
        .find_map(|state| {
            state
                .entries
                .iter()
                .find(|entry| entry.key() == receipt.destination)
                .map(|entry| (state, entry))
        })
        .ok_or_else(|| failure("retained directory origin is unavailable"))?;
    if entry.preserved_drift
        || entry.ownership.policy() != gripsack_host::store::Ownership::Owned
        || state.store_path.join(&entry.from) != receipt.predecessor_target
    {
        return Err(failure(
            "retained directory origin differs from ownership receipt",
        ));
    }
    match &entry.prior {
        None => Ok(None),
        Some(store::Prior::Symlink { target }) => Ok(Some(ObjectIdentity::Link(target.clone()))),
        Some(store::Prior::File { hash, mode }) => {
            Ok(Some(ObjectIdentity::File(store::canonical_bytes_identity(
                &store::prior::read_blob(journal.home(), hash)?,
                mode.bits(),
            ))))
        }
    }
}

fn failure(message: &str) -> ExecError {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message).into()
}
