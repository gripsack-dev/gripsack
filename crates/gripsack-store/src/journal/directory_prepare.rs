//! Exclusive node preparation precedes journaled parent publication. All
//! staging names are siblings outside the logical tree, even for nested nodes.
use super::directory_wire::{Direction, persist};
use super::{
    JournalRun,
    directory::{DirectoryIdentity, DirectoryTransition, invalid},
    directory_nodes as nodes,
};
use std::{
    collections::BTreeMap,
    io,
    path::{Component, Path, PathBuf},
};

pub fn prepare(
    destination: PathBuf,
    predecessor_target: PathBuf,
    predecessor_generation: crate::GenerationId,
    leaves: Vec<PathBuf>,
) -> io::Result<DirectoryTransition> {
    for path in [&destination, &predecessor_target] {
        if !path.is_absolute()
            || path.to_str().is_none()
            || path
                .components()
                .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        {
            return Err(invalid(
                "directory preparation requires absolute normalized UTF-8 paths",
            ));
        }
    }
    let mut paths = std::collections::BTreeSet::from([PathBuf::new()]);
    let mut unique_leaves = std::collections::BTreeSet::new();
    for leaf in &leaves {
        if leaf.as_os_str().is_empty()
            || leaf
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || !unique_leaves.insert(leaf)
        {
            return Err(invalid("unsafe or duplicate logical child destination"));
        }
        for ancestor in leaf.parent().into_iter().flat_map(Path::ancestors) {
            paths.insert(ancestor.to_owned());
        }
    }
    if leaves.is_empty() || leaves.iter().any(|leaf| paths.contains(leaf)) {
        return Err(invalid("directory leaf collides with a parent"));
    }
    let parent = gripsack_fs::open(
        destination
            .parent()
            .ok_or_else(|| invalid("missing destination parent"))?,
    )?;
    let parent_identity = DirectoryIdentity::observe(&parent, Path::new("."))?;
    let name = destination
        .file_name()
        .ok_or_else(|| invalid("missing destination name"))?;
    if parent.read_link_contents(name)? != predecessor_target {
        return Err(invalid("predecessor link changed"));
    }
    let (staging, root_identity) = nodes::prepare(&parent, &destination, Path::new(""))?;
    let mut directories = BTreeMap::from([(PathBuf::new(), root_identity)]);
    let mut directory_staging = BTreeMap::new();
    for relative in paths
        .into_iter()
        .filter(|relative| !relative.as_os_str().is_empty())
    {
        let (stage, identity) = nodes::prepare(&parent, &destination, &relative)?;
        directories.insert(relative.clone(), identity);
        directory_staging.insert(relative, stage);
    }
    let transition = DirectoryTransition {
        destination,
        predecessor_target,
        predecessor_generation,
        directories,
        directory_staging,
        borrowed_directories: BTreeMap::new(),
        leaves,
        staging,
        parent_identity,
    };
    transition.validate()?;
    Ok(transition)
}

/// A retained incarnation keeps every node identity, including when its
/// previously selected generation had a different nested-directory shape.
pub fn reactivate(
    run: &JournalRun<'_>,
    transition: &DirectoryTransition,
    expected: Option<&super::ObjectIdentity>,
) -> io::Result<()> {
    let parent = transition.parent()?;
    let name = PathBuf::from(
        transition
            .destination
            .file_name()
            .ok_or_else(|| invalid("missing destination"))?,
    );
    nodes::staged(transition)?;
    let captured = super::capture(
        run,
        parent.try_clone()?,
        name.clone(),
        &transition.destination,
        expected,
    )?;
    let mut direction = Direction::Reactivate {
        prior: super::PriorSerde::from(captured.prior()),
    };
    for previous in run
        .directories
        .read()
        .map_err(|_| invalid("directory authority lock poisoned"))?
        .iter()
    {
        if previous.transition.destination != transition.destination {
            continue;
        }
        if previous.direction != Direction::Reverse {
            return Err(invalid(
                "retained directory reactivation has an incompatible earlier transition",
            ));
        }
        direction = Direction::Exchange {
            original: previous.transition.clone(),
        };
    }
    persist(run, transition, direction)?;
    if !matches!(captured.prior(), super::Prior::Absent) {
        gripsack_fs::remove_file(&parent, &name)?;
        gripsack_fs::fsync_dir(&parent, Path::new("."))?;
    }
    nodes::install_all(transition)
}

pub fn publish(run: &JournalRun<'_>, transition: &DirectoryTransition) -> io::Result<()> {
    let parent = transition.parent()?;
    let name = transition
        .destination
        .file_name()
        .ok_or_else(|| invalid("missing destination name"))?;
    if parent.read_link_contents(name)? != transition.predecessor_target {
        return Err(invalid("predecessor link changed before cutover"));
    }
    nodes::staged(transition)?;
    persist(run, transition, Direction::Forward)?;
    gripsack_fs::remove_file(&parent, Path::new(name))?;
    gripsack_fs::fsync_dir(&parent, Path::new("."))?;
    nodes::install_all(transition)
}
