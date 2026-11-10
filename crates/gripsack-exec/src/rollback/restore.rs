//! The host driver of the kernel's rollback machine (plan/0056 F2 step 8).
//!
//! The machine (`gripsack_kernel::rollback`) decides every unit and when the
//! selection may flip; this driver observes, decodes both manifests into the
//! kernel's facts, and executes the one command the machine returns through
//! the shared planner and journal. Whole-object units (copies, templates,
//! owned links) are decided by the kernel: install the target's recorded
//! identity from retained bytes that reproduce it, release gripsack's own
//! write, leave drift, or refuse. Managed blocks (K5) and the destinations a
//! directory transition owns are delegated to the existing domain code, which
//! reports them settled or unrestorable.
use super::{DestMap, by_destination, directory_transitions, preflight};
use crate::ctx::ExecError;
use crate::deploy::Observation;
use gripsack_kernel::DestId;
use gripsack_kernel::ownership::{Recorded, Refusal as UnitRefusal};
use gripsack_kernel::recover::{Failure, Ident, Intended, Live, Route};
use gripsack_kernel::rollback::{
    Bytes, Command, Event, Place, PlanFacts, Profile, Refusal, RollbackState, Settled, Target,
    UnitFacts, rollback_step,
};
use gripsack_kernel::selection::SelectionIdentity;
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use store::hash::kernel;
use store::journal::{NoteSeverity, RecoveryNote};

/// One ownership unit and the records both generations keep for it.
struct Unit<'m> {
    key: store::OwnershipKey,
    current: Option<Side<'m>>,
    target: Option<Side<'m>>,
    route: Route,
}

/// One generation's record of a unit: its output, entry and store root.
type Side<'m> = (&'m str, &'m store::DeployedEntry, &'m Path);

/// How the host's own domain settled a delegated unit.
enum Delegation {
    Settled,
    Unrestorable(String),
}

/// Restore every unit through the kernel machine up to its flip. Returns the
/// machine state and the selection it reselects; the caller prepares
/// activation, flips, and answers with [`finish`].
pub(super) fn until_flip(
    journal: &store::journal::JournalRun<'_>,
    current: Option<&store::Generation>,
    target: &store::Generation,
    notes: &mut Vec<RecoveryNote>,
) -> Result<(RollbackState, SelectionIdentity), ExecError> {
    let home = journal.home();
    let home_path = journal.home_path();
    preflight(target)?;
    let current_directories = current
        .map(|generation| generation.directory_transitions.as_slice())
        .unwrap_or_default();
    let mut predecessors = BTreeMap::new();
    for transition in current_directories {
        if let std::collections::btree_map::Entry::Vacant(slot) =
            predecessors.entry(transition.predecessor_generation)
        {
            let predecessor =
                store::generations::admit_manifest(home_path, transition.predecessor_generation)?;
            preflight(&predecessor)?;
            slot.insert(predecessor);
        }
    }
    let mut current_by_dest = current.map(by_destination).unwrap_or_default();
    for transition in current_directories {
        let predecessor = &predecessors[&transition.predecessor_generation];
        let (name, state, entry) = predecessor
            .deployments
            .iter()
            .find_map(|(name, state)| {
                state
                    .entries
                    .iter()
                    .find(|entry| entry.key() == transition.destination)
                    .map(|entry| (name, state, entry))
            })
            .ok_or_else(|| {
                std::io::Error::other("directory transition lost its predecessor ownership lineage")
            })?;
        if state.store_path.join(&entry.from) != transition.predecessor_target
            || entry.preserved_drift
            || entry.ownership.policy() != gripsack_store::Ownership::Owned
        {
            return Err(std::io::Error::other(
                "directory predecessor lineage no longer matches receipt",
            )
            .into());
        }
        current_by_dest.insert(entry.ownership_key(name), (name, entry, &state.store_path));
    }
    let target_by_dest = by_destination(target);
    let mut exchanged =
        directory_transitions::prepare(journal, &current_by_dest, current_directories, target)?;
    notes.append(&mut exchanged.notes);
    let units = plan_units(
        &current_by_dest,
        &target_by_dest,
        current_directories,
        target,
        &exchanged.released,
    );
    let plan = PlanFacts {
        units: u32::try_from(units.len())
            .map_err(|_| std::io::Error::other("too many rollback units"))?,
        reselect: *journal.identity(),
        previous: store::generations::current_selection_in(home_path, home)?,
        profile: if crate::env::retains_profile(home_path, target.number, &target.deployments)? {
            Profile::Retained
        } else {
            Profile::Missing
        },
    };

    let mut state = RollbackState::new();
    let mut event = Event::Start;
    // the unit under decision's one observation, read with its facts
    let mut observed: Option<Option<Observation>> = None;
    // why the host's domain could not restore a delegated unit
    let mut unrestorable: Option<String> = None;
    loop {
        let (next, command) = rollback_step(state, event);
        state = next;
        event = match command {
            Command::ReadPlan => Event::PlanRead(plan),
            Command::ReadUnit(index) => {
                let unit = unit_at(&units, index)?;
                let (facts, observation) = unit_facts(index, unit, home, home_path)?;
                observed = observation;
                Event::UnitRead(facts)
            }
            Command::Observe { dest, .. } => Event::Observed {
                dest,
                live: match &observed {
                    Some(observation) => crate::ops::kernel_live(observation.as_ref()),
                    None => Live::Unsupported,
                },
            },
            Command::Mutate { unit, .. } => mutate(journal, unit_at(&units, unit)?, home_path)?,
            Command::Delegate { unit, .. } => {
                let unit = unit_at(&units, unit)?;
                let released = &exchanged.released;
                match settle_delegated(journal, unit, current_directories, target, released, notes)?
                {
                    Delegation::Settled => Event::Done,
                    Delegation::Unrestorable(why) => {
                        unrestorable = Some(why);
                        Event::Refused
                    }
                }
            }
            Command::Report { unit, settled, .. } => {
                report(notes, unit_at(&units, unit)?, settled);
                Event::Done
            }
            Command::Flip(reselect) => return Ok((state, reselect)),
            Command::Finish(_) => {
                return Err(protocol("the rollback machine finished before its flip"));
            }
            Command::Abort(refusal) => {
                return Err(refused(refusal, &units, target, unrestorable.take()));
            }
        };
    }
}

/// Answer the flip: the machine must finish.
pub(super) fn finish(state: RollbackState) -> Result<(), ExecError> {
    match rollback_step(state, Event::Done).1 {
        Command::Finish(_) => Ok(()),
        _ => Err(protocol(
            "the rollback machine did not finish after its flip",
        )),
    }
}

fn protocol(detail: &str) -> ExecError {
    ExecError::Step {
        output: "*".into(),
        step: "rollback".into(),
        detail: detail.into(),
    }
}

fn unit_at<'u, 'm>(units: &'u [Unit<'m>], index: u32) -> Result<&'u Unit<'m>, ExecError> {
    usize::try_from(index)
        .ok()
        .and_then(|index| units.get(index))
        .ok_or_else(|| protocol("the rollback machine named a unit outside its plan"))
}

/// Every destination either generation deploys, deepest first (children
/// before the parents a directory transition exchanges), routed to the
/// kernel or to the host's own domain.
fn plan_units<'m>(
    current_by_dest: &DestMap<'m>,
    target_by_dest: &DestMap<'m>,
    current_directories: &[store::journal::directory::DirectoryTransition],
    target: &store::Generation,
    released: &BTreeSet<store::OwnershipKey>,
) -> Vec<Unit<'m>> {
    // A spelling only one generation records may name the same object as a
    // spelling only the other records (a case-only re-spelling on a
    // case-insensitive mount, plan/0056 FND-3): that is one unit, under the
    // target's spelling — never a release of the object the target restores.
    let mut aliases: BTreeMap<store::OwnershipKey, store::OwnershipKey> = BTreeMap::new();
    for (key, (_, entry, _)) in target_by_dest {
        if current_by_dest.contains_key(key) {
            continue;
        }
        if let Some(alias) = current_by_dest
            .iter()
            .find_map(|(other, (name, current, _))| {
                let block = (current.ownership.policy() == gripsack_store::Ownership::Merge)
                    .then(|| current.merge_owner(name));
                (!target_by_dest.contains_key(other)
                    && entry.ownership.policy() == current.ownership.policy()
                    && store::OwnershipKey::new(key.destination().to_path_buf(), block) == *key
                    && crate::apply::same_destination(other.destination(), key.destination()))
                .then(|| other.clone())
            })
        {
            aliases.insert(key.clone(), alias);
        }
    }
    let aliased: BTreeSet<&store::OwnershipKey> = aliases.values().collect();
    let mut keys: Vec<store::OwnershipKey> = current_by_dest
        .keys()
        .filter(|key| !aliased.contains(key))
        .chain(target_by_dest.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    keys.sort_by_key(|key| std::cmp::Reverse(key.destination().components().count()));
    keys.into_iter()
        .map(|key| {
            let current = current_by_dest
                .get(aliases.get(&key).unwrap_or(&key))
                .copied();
            let target_side = target_by_dest.get(&key).copied();
            let directory = |transitions: &[store::journal::directory::DirectoryTransition]| {
                transitions
                    .iter()
                    .any(|transition| transition.destination == key.destination())
            };
            let merge = |side: Option<Side<'_>>| {
                side.is_some_and(|(_, entry, _)| {
                    entry.ownership.policy() == gripsack_store::Ownership::Merge
                })
            };
            let delegated = directory(&target.directory_transitions)
                || directory(current_directories)
                || (target_side.is_none() && released.contains(&key))
                || merge(current)
                || merge(target_side);
            Unit {
                key,
                current,
                target: target_side,
                route: if delegated {
                    Route::Delegated
                } else {
                    Route::Leaf
                },
            }
        })
        .collect()
}

/// One observation of a destination: absent, a file or link, or an object
/// rollback cannot express (a directory, a special file).
fn observe(dest: &Path) -> Result<Option<Option<Observation>>, ExecError> {
    match crate::deploy::observe_readonly(dest) {
        Ok(observation) => Ok(Some(observation)),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => Ok(None),
        Err(error) => Err(ExecError::Step {
            output: "*".into(),
            step: "rollback".into(),
            detail: format!("cannot inspect {}: {error}", dest.display()),
        }),
    }
}

/// Decode one unit into the kernel's facts, with the observation they were
/// decided from.
fn unit_facts(
    index: u32,
    unit: &Unit<'_>,
    home: &gripsack_fs::Dir,
    home_path: &Path,
) -> Result<(UnitFacts, Option<Option<Observation>>), ExecError> {
    let dest = unit.key.destination();
    let observation = match unit.route {
        Route::Leaf => observe(dest)?,
        Route::Delegated => None,
    };
    let seen = observation.as_ref().and_then(Option::as_ref);
    let current = match unit.current {
        None => Recorded::Unrecorded,
        Some((_, entry, store_path)) => {
            crate::ops::PreviousRecord { entry, store_path }.recorded(seen)
        }
    };
    let target = match (unit.route, unit.target) {
        // the host's domain settles it; the kernel never decides from this
        (Route::Delegated, _) => Target::Preserved,
        (Route::Leaf, None) => Target::Released(release(unit.current, home)?),
        (Route::Leaf, Some((_, entry, _))) if entry.preserved_drift => Target::Preserved,
        (Route::Leaf, Some((name, entry, store_path))) => {
            match crate::ops::retained(name, entry, store_path)? {
                crate::ops::Retained::File { content, mode } => Target::Wrote {
                    ident: kernel::file(&store::canonical_bytes_identity(&content, mode)),
                    bytes: Bytes::Retained,
                },
                crate::ops::Retained::Link { source, .. } => Target::Wrote {
                    ident: kernel::link(source.as_os_str().as_encoded_bytes()),
                    bytes: Bytes::Retained,
                },
                crate::ops::Retained::Missing => Target::Wrote {
                    ident: recorded_identity(name, entry, store_path, seen)?,
                    bytes: Bytes::Missing,
                },
            }
        }
    };
    let generations = home_path.join("generations");
    let inside = dest.starts_with(&generations)
        || std::fs::canonicalize(&generations).is_ok_and(|real| dest.starts_with(real));
    Ok((
        UnitFacts {
            dest: DestId::new(index),
            place: if inside {
                Place::Generation
            } else {
                Place::Outside
            },
            route: unit.route,
            current,
            target,
        },
        observation,
    ))
}

/// What the target recorded, when no retained bytes reproduce it: the live
/// object if it already is that record (nothing to write), else the record's
/// own identity.
fn recorded_identity(
    output: &str,
    entry: &store::DeployedEntry,
    store_path: &Path,
    seen: Option<&Observation>,
) -> Result<Ident, ExecError> {
    if entry.ownership.policy() == gripsack_store::Ownership::Owned {
        return Ok(kernel::link(
            store_path.join(&entry.from).as_os_str().as_encoded_bytes(),
        ));
    }
    if let Some(Observation::File { bytes, mode }) = seen
        && entry.matches_file(bytes, *mode)
    {
        return Ok(kernel::file(&store::canonical_bytes_identity(bytes, *mode)));
    }
    kernel::recorded_file(&entry.hash).ok_or_else(|| ExecError::Step {
        output: output.to_string(),
        step: "rollback".into(),
        detail: format!(
            "{}: the recorded identity is not a content digest — refusing to roll back",
            entry.to
        ),
    })
}

/// What releasing a destination brings back: the object the ownership epoch
/// recorded before gripsack took it over, or nothing (apply's prune intent).
fn release(current: Option<Side<'_>>, home: &gripsack_fs::Dir) -> Result<Intended, ExecError> {
    let Some((_, entry, _)) = current else {
        return Ok(Intended::Removed);
    };
    if entry.preserved_drift {
        // preserved drift is never released (the kernel refuses), whatever
        // it recorded
        return Ok(Intended::Removed);
    }
    Ok(match &entry.prior {
        None => Intended::Removed,
        Some(store::Prior::File { hash, mode }) => {
            let bytes = store::prior::read_blob(home, hash)?;
            Intended::Obj(kernel::file(&store::canonical_bytes_identity(
                &bytes,
                mode.bits(),
            )))
        }
        Some(store::Prior::Symlink { target }) => Intended::Obj(kernel::link(target.as_bytes())),
    })
}

/// Execute the machine's write through the shared planner and journal (whose
/// re-check pins the observed object), then observe what landed. The planner
/// decides with the same kernel function; if it no longer agrees, the
/// destination changed since the machine observed it.
fn mutate(
    journal: &store::journal::JournalRun<'_>,
    unit: &Unit<'_>,
    home_path: &Path,
) -> Result<Event, ExecError> {
    let op = match unit.target {
        Some((name, entry, store_path)) => crate::ops::plan_restore_op(
            name,
            entry,
            store_path,
            unit.current
                .map(|(_, entry, store_path)| crate::ops::PreviousRecord { entry, store_path }),
            home_path,
        )?,
        None => match unit.current {
            Some((name, entry, store_path)) => {
                crate::ops::plan_remove_op(name, entry, store_path, journal.home())?
            }
            None => None,
        },
    };
    let Some(op) = op else {
        return Ok(Event::Failed(Failure::Drift));
    };
    if matches!(
        op.kind(),
        crate::ops::OpKind::Satisfied | crate::ops::OpKind::Preserved
    ) {
        return Ok(Event::Failed(Failure::Drift));
    }
    crate::ops::execute_op(journal, op.as_executable()?)?;
    let live = match observe(unit.key.destination())? {
        Some(observation) => crate::ops::kernel_live(observation.as_ref()),
        None => Live::Unsupported,
    };
    Ok(Event::Landed { live })
}

/// The note a settled unit leaves (the drift kept for the user, by K1).
fn report(notes: &mut Vec<RecoveryNote>, unit: &Unit<'_>, settled: Settled) {
    let dest = unit.key.destination().display();
    let message = match settled {
        Settled::Kept(UnitRefusal::Unsupported) => {
            format!("kept {dest} — not a file or link rollback can restore; it stands")
        }
        Settled::Kept(_) if unit.current.is_some() => {
            format!("kept {dest} — drifted since the current generation; your edit stands")
        }
        Settled::Kept(_) => format!("kept {dest} — foreign content stands there; your edit stands"),
        Settled::Restored | Settled::Already | Settled::Untouched | Settled::Delegated => return,
    };
    notes.push(RecoveryNote {
        severity: NoteSeverity::Warn,
        message,
    });
}

/// A unit the host's own domain settles: a managed block (K5), or a
/// destination a directory transition owns. Unrestorable is reported, never
/// skipped: the machine refuses the rollback before it flips.
fn settle_delegated(
    journal: &store::journal::JournalRun<'_>,
    unit: &Unit<'_>,
    current_directories: &[store::journal::directory::DirectoryTransition],
    target: &store::Generation,
    released: &BTreeSet<store::OwnershipKey>,
    notes: &mut Vec<RecoveryNote>,
) -> Result<Delegation, ExecError> {
    let home = journal.home();
    let home_path = journal.home_path();
    let dest = &unit.key;
    if target
        .directory_transitions
        .iter()
        .any(|transition| transition.destination == dest.destination())
    {
        // The directory domain, not a flat unlink, owns this old link.
        return Ok(Delegation::Settled);
    }
    if let Some(transition) = current_directories
        .iter()
        .find(|transition| transition.destination == dest.destination())
        && !target
            .directory_transitions
            .iter()
            .any(|target| target.destination == transition.destination)
    {
        store::journal::reverse_directory(journal, transition)?;
    }
    let unrestorable = |entry: &store::DeployedEntry| {
        Delegation::Unrestorable(match entry.ownership.policy() {
            gripsack_store::Ownership::Merge => format!(
                "cannot restore the managed block in {}: the file is no longer text a block can \
                 be spliced into (or its block payload is gone)",
                dest.destination().display()
            ),
            _ => format!(
                "cannot restore {} exactly: no retained bytes reproduce generation {}'s record",
                dest.destination().display(),
                target.number
            ),
        })
    };
    match (unit.current, unit.target) {
        // only the current generation deploys it: the prune rule
        (Some((name, entry, store_path)), None) => {
            // preserved drift was never written by gripsack — rollback and
            // prune never touch it (0029 §2); a released key belongs to the
            // directory exchange
            if entry.preserved_drift || released.contains(dest) {
                return Ok(Delegation::Settled);
            }
            match crate::ops::plan_remove_op(name, entry, store_path, home)? {
                None => {}
                Some(op) if matches!(op.kind(), crate::ops::OpKind::Preserved) => {
                    notes.push(RecoveryNote {
                        severity: NoteSeverity::Warn,
                        message: format!(
                            "kept {} — drifted since the current generation; your edit stands",
                            op.dest().display()
                        ),
                    });
                }
                Some(op) => {
                    crate::ops::execute_op(journal, op.as_executable()?)?;
                }
            }
            Ok(Delegation::Settled)
        }
        (current, Some((name, entry, store_path))) => {
            if entry.preserved_drift || current.is_some_and(|(_, entry, _)| entry.preserved_drift) {
                return Ok(Delegation::Settled);
            }
            let lineage = current
                .map(|(_, entry, store_path)| crate::ops::PreviousRecord { entry, store_path });
            match crate::ops::plan_restore_op(name, entry, store_path, lineage, home_path)? {
                None => Ok(unrestorable(entry)),
                Some(op) => {
                    match op.kind() {
                        crate::ops::OpKind::Satisfied => {}
                        crate::ops::OpKind::Preserved => notes.push(RecoveryNote {
                            severity: NoteSeverity::Warn,
                            message: if current.is_some() {
                                format!(
                                    "kept {} — drifted since the current generation; your edit stands",
                                    op.dest().display()
                                )
                            } else {
                                format!(
                                    "kept {} — foreign content stands there; your edit stands",
                                    op.dest().display()
                                )
                            },
                        }),
                        _ => {
                            crate::ops::execute_op(journal, op.as_executable()?)?;
                        }
                    }
                    Ok(Delegation::Settled)
                }
            }
        }
        (None, None) => Ok(Delegation::Settled), // unreachable: union of keys
    }
}

/// Why the rollback stopped, said for the user. Nothing was reselected; the
/// caller compensates every write this run made from the journal.
fn refused(
    refusal: Refusal,
    units: &[Unit<'_>],
    target: &store::Generation,
    unrestorable: Option<String>,
) -> ExecError {
    let at = |unit: u32| {
        usize::try_from(unit)
            .ok()
            .and_then(|index| units.get(index))
            .map_or_else(
                || format!("unit {unit}"),
                |unit| unit.key.destination().display().to_string(),
            )
    };
    let generation = target.number;
    let detail = match refusal {
        Refusal::ProfileMissing => format!(
            "generation {generation} has no shell profile of its own (it predates \
             in-generation profiles); restoring it would mean writing into an existing \
             generation — refusing to roll back. Re-apply that configuration instead"
        ),
        Refusal::Reused => {
            "the reserved selection does not carry a fresh transaction — refusing to roll back"
                .into()
        }
        Refusal::Unrestorable { unit, .. } => match unrestorable {
            Some(why) => {
                format!("{why} — refusing to roll back; the current generation stays selected")
            }
            None => format!(
                "cannot restore {} exactly: no retained bytes reproduce what generation \
                 {generation} recorded there (an older receipt without its mode, or a template \
                 whose recorded variables no longer render it) — refusing to roll back; the \
                 current generation stays selected",
                at(unit)
            ),
        },
        Refusal::Protected { unit, .. } => format!(
            "refusing to write {}: it lies inside a published generation's directory",
            at(unit)
        ),
        Refusal::Drifted { unit, .. } => format!(
            "{} changed while rolling back — refusing; the current generation stays selected",
            at(unit)
        ),
        Refusal::NotRestored { unit, .. } => format!(
            "{} did not land generation {generation}'s recorded identity — refusing to roll back",
            at(unit)
        ),
        Refusal::Malformed | Refusal::Failed | Refusal::Protocol => {
            format!("the rollback machine stopped ({refusal:?})")
        }
    };
    ExecError::Step {
        output: "*".into(),
        step: "rollback".into(),
        detail,
    }
}
