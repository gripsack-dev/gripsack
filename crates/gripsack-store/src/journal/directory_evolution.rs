//! Ordinary declarations extend ancestor authority without acquiring foreign
//! contents. Exclusive node births are prepared before any old leaf is released.
use super::directory_wire::{Direction, DirectoryRecord, persist};
use super::{
    JournalRun,
    directory::{DirectoryIdentity, DirectoryTransition, invalid},
    directory_nodes as nodes,
};
use std::{
    collections::BTreeSet,
    io,
    path::{Component, Path, PathBuf},
};

pub struct EvolutionPlan {
    before: DirectoryRecord,
    target: DirectoryTransition,
    blockers: Vec<PathBuf>,
    prepared: Vec<PathBuf>,
    registered: bool,
}

impl EvolutionPlan {
    /// These paths require ordinary per-file ownership admission. Directory
    /// planning does not authorize their deletion, even if they block a birth.
    pub fn blockers(&self) -> &[PathBuf] {
        &self.blockers
    }

    pub fn publish(mut self, run: &JournalRun<'_>) -> io::Result<()> {
        if self.target == self.before.transition {
            return Ok(());
        }
        persist_change(run, &self.before, &self.target)?;
        self.registered = true;
        super::directory_shape::apply(&self.before.transition, &self.target)
    }
}

impl Drop for EvolutionPlan {
    fn drop(&mut self) {
        if self.registered {
            return;
        }
        // Only private, exact empty preparation objects are eligible. A failed
        // cleanup leaves the object intact; there is never recursive deletion.
        if let Ok(parent) = self.before.transition.parent() {
            for relative in &self.prepared {
                if let Some(stage) = self
                    .target
                    .directory_staging
                    .get(relative)
                    .and_then(|path| path.file_name())
                {
                    let _ = nodes::discard_in(
                        &parent,
                        Path::new(stage),
                        self.target.directories[relative],
                    );
                }
            }
        }
    }
}

pub fn plans(run: &JournalRun<'_>, destinations: &[PathBuf]) -> io::Result<Vec<EvolutionPlan>> {
    let records = run
        .directories
        .read()
        .map_err(|_| invalid("directory authority lock poisoned"))?
        .clone();
    let mut plans = Vec::new();
    for before in records {
        if before.direction == Direction::Reverse {
            continue;
        }
        let desired: BTreeSet<PathBuf> = destinations
            .iter()
            .filter_map(|destination| {
                destination
                    .strip_prefix(&before.transition.destination)
                    .ok()
                    .filter(|relative| !relative.as_os_str().is_empty())
                    .map(Path::to_owned)
            })
            .collect();
        if desired.is_empty() {
            continue;
        }
        let mut needed = BTreeSet::from([PathBuf::new()]);
        for leaf in &desired {
            if leaf
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err(invalid("unsafe evolution leaf"));
            }
            for parent in leaf.parent().into_iter().flat_map(Path::ancestors) {
                needed.insert(parent.to_owned());
            }
        }
        if desired.iter().any(|leaf| needed.contains(leaf)) {
            return Err(invalid("file and directory declarations overlap"));
        }
        let mut plan = EvolutionPlan {
            target: before.transition.clone(),
            before,
            blockers: Vec::new(),
            prepared: Vec::new(),
            registered: false,
        };
        let replacing: Vec<_> = desired
            .iter()
            .filter(|leaf| plan.target.directories.contains_key(*leaf))
            .cloned()
            .collect();
        if desired
            .iter()
            .any(|leaf| plan.target.borrowed_directories.contains_key(leaf))
        {
            return Err(invalid(
                "a borrowed directory cannot be replaced by a declared file",
            ));
        }
        for relative in &replacing {
            plan.blockers.push(plan.target.destination.join(relative));
        }
        plan.target
            .directories
            .retain(|relative, _| !replacing.iter().any(|root| relative.starts_with(root)));
        plan.target
            .directory_staging
            .retain(|relative, _| plan.target.directories.contains_key(relative));
        plan.target
            .borrowed_directories
            .retain(|relative, _| !replacing.iter().any(|root| relative.starts_with(root)));
        plan.target.leaves.retain(|leaf| {
            !needed.contains(leaf) && !replacing.iter().any(|root| leaf.starts_with(root))
        });
        let outer = plan.before.transition.parent()?;
        let root_name = Path::new(
            plan.target
                .destination
                .file_name()
                .ok_or_else(|| invalid("missing root name"))?,
        );
        let root = plan.target.directories[Path::new("")].open_verified(&outer, root_name)?;
        let mut ordered: Vec<_> = needed
            .into_iter()
            .filter(|path| !path.as_os_str().is_empty())
            .collect();
        ordered.sort_by_key(|path| path.components().count());
        for relative in ordered {
            if plan.target.directories.contains_key(&relative)
                || plan.target.borrowed_directories.contains_key(&relative)
            {
                continue;
            }
            let absent_parent = plan
                .prepared
                .iter()
                .any(|ancestor| relative.starts_with(ancestor));
            let existing = if absent_parent {
                None
            } else {
                let destination = plan.target.destination.join(&relative);
                let parent = plan
                    .target
                    .open_relative(&root, relative.parent().unwrap_or(Path::new("")))?;
                let name = Path::new(
                    relative
                        .file_name()
                        .ok_or_else(|| invalid("missing parent component"))?,
                );
                match parent.symlink_metadata(name) {
                    Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                        Some(DirectoryIdentity::observe(&parent, name)?)
                    }
                    Ok(_) => {
                        plan.blockers.push(destination);
                        None
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error),
                }
            };
            if let Some(identity) = existing {
                plan.target.borrowed_directories.insert(relative, identity);
            } else {
                let (stage, identity) =
                    nodes::prepare(&outer, &plan.target.destination, &relative)?;
                plan.target.directories.insert(relative.clone(), identity);
                plan.target
                    .directory_staging
                    .insert(relative.clone(), stage);
                plan.prepared.push(relative);
            }
        }
        plan.target.leaves.extend(desired);
        plan.target.leaves.sort();
        plan.target.leaves.dedup();
        plan.target.validate()?;
        plans.push(plan);
    }
    Ok(plans)
}

pub(super) fn persist_change(
    run: &JournalRun<'_>,
    before: &DirectoryRecord,
    target: &DirectoryTransition,
) -> io::Result<()> {
    let direction = match &before.direction {
        Direction::Bound => Direction::Evolve {
            original: before.transition.clone(),
        },
        Direction::Evolve { original } => Direction::Evolve {
            original: original.clone(),
        },
        Direction::Forward => Direction::Forward,
        Direction::Reactivate { prior } => Direction::Reactivate {
            prior: prior.clone(),
        },
        Direction::Exchange { original } => Direction::Exchange {
            original: original.clone(),
        },
        Direction::Reverse => return Err(invalid("cannot evolve a reversed directory")),
    };
    persist(run, target, direction)
}

pub fn receipts(run: &JournalRun<'_>) -> io::Result<Vec<DirectoryTransition>> {
    Ok(run
        .directories
        .read()
        .map_err(|_| invalid("directory authority lock poisoned"))?
        .iter()
        .filter(|record| record.direction != Direction::Reverse)
        .map(|record| record.transition.clone())
        .collect())
}

pub fn is_directory(run: &JournalRun<'_>, destination: &Path) -> io::Result<bool> {
    Ok(run
        .directories
        .read()
        .map_err(|_| invalid("directory authority lock poisoned"))?
        .iter()
        .any(|record| {
            destination
                .strip_prefix(&record.transition.destination)
                .is_ok_and(|relative| record.transition.directories.contains_key(relative))
        }))
}

/// Called after ordinary leaf pruning. Nonempty foreign children preserve their
/// containing nodes; only exact empty owned nodes are retired into history.
pub fn finish(
    run: &JournalRun<'_>,
    destination: &Path,
    leaves: Vec<PathBuf>,
) -> io::Result<DirectoryTransition> {
    let before = run
        .directories
        .read()
        .map_err(|_| invalid("directory authority lock poisoned"))?
        .iter()
        .find(|record| record.transition.destination == destination)
        .cloned()
        .ok_or_else(|| invalid("missing directory evolution authority"))?;
    let mut target = before.transition.clone();
    target.leaves = leaves;
    target.leaves.sort();
    target.leaves.dedup();
    let mut required = BTreeSet::from([PathBuf::new()]);
    for leaf in &target.leaves {
        for parent in leaf.parent().into_iter().flat_map(Path::ancestors) {
            required.insert(parent.to_owned());
        }
    }
    let mut retiring = BTreeSet::new();
    for relative in nodes::ordered(&before.transition, true) {
        if required.contains(relative) {
            continue;
        }
        let path = before.transition.destination.join(relative);
        let (parent, name) = before.transition.leaf_parent(&path)?;
        let directory = before.transition.directories[relative].open_verified(&parent, &name)?;
        let mut empty_after_children = true;
        for entry in directory.read_dir(".")? {
            if !retiring.contains(&relative.join(entry?.file_name())) {
                empty_after_children = false;
                break;
            }
        }
        if empty_after_children {
            retiring.insert(relative.clone());
        }
    }
    target
        .directories
        .retain(|relative, _| !retiring.contains(relative));
    target
        .directory_staging
        .retain(|relative, _| target.directories.contains_key(relative));
    target.borrowed_directories.retain(|relative, _| {
        target.leaves.iter().any(|leaf| leaf.starts_with(relative))
            || target
                .directories
                .keys()
                .any(|owned| owned.starts_with(relative))
    });
    target.validate()?;
    if target != before.transition {
        persist_change(run, &before, &target)?;
        super::directory_shape::apply(&before.transition, &target)?;
    }
    Ok(target)
}

pub fn restore_shape(run: &JournalRun<'_>, target: &DirectoryTransition) -> io::Result<()> {
    let before = run
        .directories
        .read()
        .map_err(|_| invalid("directory authority lock poisoned"))?
        .iter()
        .find(|record| record.transition.destination == target.destination)
        .cloned()
        .ok_or_else(|| invalid("missing current directory shape"))?;
    if before.transition == *target {
        return Ok(());
    }
    persist_change(run, &before, target)?;
    super::directory_shape::apply(&before.transition, target)
}
