//! Resolve source aliases without granting access to ambient ancestor trees.
use super::inventory::{MAX_DEPTH, MAX_LINK_EXPANSIONS, invalid};
use super::policy::CaptureAdmission;
use super::{CaptureBudget, CaptureRoot};
use gripsack_fs::Dir;
use std::{
    collections::VecDeque,
    ffi::OsString,
    io,
    path::{Component, Path, PathBuf},
};

#[derive(Debug)]
enum Part {
    Root,
    Parent,
    Current,
    Name(OsString),
    RequireDirectory,
}

fn parts(path: &Path) -> io::Result<VecDeque<Part>> {
    let text = path
        .to_str()
        .ok_or_else(|| invalid("source alias target is not UTF-8"))?;
    if text.is_empty() || text.contains('\0') {
        return Err(invalid("source alias target is empty or contains NUL"));
    }
    let mut result = VecDeque::new();
    if path.is_absolute() {
        result.push_back(Part::Root);
    }
    for component in text.split('/') {
        match component {
            "" => {}
            "." => result.push_back(Part::Current),
            ".." => result.push_back(Part::Parent),
            name => result.push_back(Part::Name(name.into())),
        }
    }
    if text.ends_with('/') {
        result.push_back(Part::RequireDirectory);
    }
    Ok(result)
}

fn endpoint(roots: &[CaptureRoot], path: &Path) -> Option<usize> {
    roots.iter().enumerate().rev().find_map(|(index, root)| {
        (path == root.canonical
            || path == root.declared
            || root.declared_alias.as_deref() == Some(path))
        .then_some(index)
    })
}

fn is_root_ancestor(roots: &[CaptureRoot], path: &Path) -> bool {
    roots.iter().any(|root| {
        root.canonical.starts_with(path)
            || root.declared.starts_with(path)
            || root
                .declared_alias
                .as_ref()
                .is_some_and(|alias| alias.starts_with(path))
    })
}

/// An unresolved suffix is never lexically collapsed across a symlink. Outside
/// directory names may appear only as virtual prefixes of an already-held
/// root capability; no filesystem operation is issued through those prefixes.
pub(super) fn resolve(
    roots: &[CaptureRoot],
    from_root: usize,
    parent: &Path,
    target: &Path,
    admission: &CaptureAdmission<'_>,
    budget: &mut CaptureBudget,
) -> io::Result<PathBuf> {
    let origin = roots
        .get(from_root)
        .ok_or_else(|| invalid("unknown source root"))?;
    let mut selected = Some(from_root);
    let mut relative = PathBuf::new();
    let mut physical = origin.canonical.clone();
    let mut directories: Vec<Dir> = Vec::new();
    // The copying walker supplies a real directory path. Pin every component
    // again for this resolution; a substituted alias is refused, not followed.
    for component in parent.components() {
        let Component::Normal(name) = component else {
            return Err(invalid("source alias parent is not root-relative"));
        };
        budget.resolve_step()?;
        admission.require_available(origin, &relative.join(name))?;
        let directory = directories.last().unwrap_or(&origin.directory);
        let child = gripsack_fs::open_dir_nofollow(directory, Path::new(name))?;
        directories.push(child);
        relative.push(name);
        physical.push(name);
    }
    let mut pending = parts(target)?;
    let mut expansions = 0_usize;
    while let Some(part) = pending.pop_front() {
        budget.resolve_step()?;
        match part {
            Part::Root => {
                selected = None;
                relative.clear();
                directories.clear();
                physical = PathBuf::from("/");
                if let Some(index) = endpoint(roots, &physical) {
                    selected = Some(index);
                    physical = roots[index].canonical.clone();
                }
            }
            Part::Current | Part::RequireDirectory => {}
            Part::Parent => {
                if selected.is_some() && !relative.as_os_str().is_empty() {
                    relative.pop();
                    physical.pop();
                    directories.pop();
                } else {
                    selected = None;
                    directories.clear();
                    relative.clear();
                    physical.pop();
                    if physical.as_os_str().is_empty() {
                        physical.push("/");
                    }
                    if let Some(index) = endpoint(roots, &physical) {
                        selected = Some(index);
                        physical = roots[index].canonical.clone();
                    } else if !is_root_ancestor(roots, &physical) {
                        return Err(invalid("source alias escapes its admitted roots"));
                    }
                }
            }
            Part::Name(name) => {
                let candidate = physical.join(&name);
                // Check before endpoint promotion or any filesystem read. A
                // symlink cannot turn an excluded spelling into a root grant.
                if let Some(index) = selected {
                    admission.require_available(&roots[index], &relative.join(&name))?;
                }
                if let Some(index) = endpoint(roots, &candidate)
                    && (selected != Some(index) || !relative.as_os_str().is_empty())
                {
                    selected = Some(index);
                    physical = roots[index].canonical.clone();
                    relative.clear();
                    directories.clear();
                    continue;
                }
                let Some(index) = selected else {
                    if !is_root_ancestor(roots, &candidate) {
                        return Err(invalid("source alias escapes its admitted roots"));
                    }
                    physical = candidate;
                    continue;
                };
                let directory = directories.last().unwrap_or(&roots[index].directory);
                let metadata = directory.symlink_metadata(Path::new(&name))?;
                if metadata.file_type().is_symlink() {
                    expansions = expansions
                        .checked_add(1)
                        .ok_or_else(|| invalid("source alias expansion overflow"))?;
                    if expansions > MAX_LINK_EXPANSIONS {
                        return Err(invalid(
                            "source alias has a cycle or exceeds its expansion budget",
                        ));
                    }
                    let target = directory.read_link_contents(Path::new(&name))?;
                    let mut replacement = parts(&target)?;
                    replacement.append(&mut pending);
                    pending = replacement;
                    continue;
                }
                if !metadata.is_dir() && !metadata.is_file() {
                    return Err(invalid("source alias targets a special file"));
                }
                relative.push(&name);
                physical = candidate;
                if relative.components().count() + 1 > MAX_DEPTH {
                    return Err(invalid("source alias exceeds its path-depth budget"));
                }
                if metadata.is_file() {
                    if !pending.is_empty() {
                        return Err(io::Error::new(
                            io::ErrorKind::NotADirectory,
                            "source alias traverses a regular file",
                        ));
                    }
                    return Ok(roots[index].logical(&relative));
                }
                let child = gripsack_fs::open_dir_nofollow(directory, Path::new(&name))?;
                directories.push(child);
            }
        }
    }
    let index = selected.ok_or_else(|| invalid("source alias does not name an admitted object"))?;
    Ok(roots[index].logical(&relative))
}
