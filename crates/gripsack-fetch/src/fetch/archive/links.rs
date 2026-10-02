//! Complete payload link graph admission, independent of member order.
//!
//! A lexical `..` check is insufficient: `d/up -> ..` followed by
//! `leak -> d/up/../sentinel` crosses the root only after resolving
//! the first link. Directory names here include implicit parents, so
//! forward targets and archive order have the same result.

use crate::{FetchError, FetchLimits};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use super::paths;

/// Bound recursion even if the entry cap permits a very long link chain.
const MAX_LINK_HOPS: usize = 64;

#[derive(Debug)]
pub(super) enum Kind {
    Directory { explicit: bool },
    Regular,
    Symlink(PathBuf),
    HardLink(PathBuf),
}

pub(super) struct Graph {
    nodes: BTreeMap<PathBuf, Kind>,
    entry_limit: usize,
    entries: usize,
    metadata_limit: u64,
    metadata_bytes: u64,
}

impl Graph {
    pub(super) fn new(limits: FetchLimits) -> Self {
        Self {
            nodes: BTreeMap::new(),
            entry_limit: limits.archive_entries.get(),
            entries: 0,
            metadata_limit: limits.decoder_bytes.get().min(64 * 1024 * 1024),
            metadata_bytes: 0,
        }
    }

    fn charge(&mut self, bytes: usize) -> Result<(), FetchError> {
        self.metadata_bytes = self
            .metadata_bytes
            .checked_add(bytes as u64 + 128)
            .filter(|total| *total <= self.metadata_limit)
            .ok_or_else(|| FetchError::PayloadTooLarge {
                what: "archive link graph metadata".into(),
                limit: self.metadata_limit,
            })?;
        Ok(())
    }

    /// `path` has already passed the archive's relative-name admission.
    /// Store implicit parents so resolving a target does not depend on
    /// whether a directory had an explicit member or was emitted later.
    pub(super) fn insert(&mut self, path: PathBuf, kind: Kind) -> Result<(), FetchError> {
        if path.as_os_str().is_empty() {
            return Err(paths::violation(
                &path,
                "entry does not name a payload child",
            ));
        }
        if self.entries == self.entry_limit {
            return Err(FetchError::TooManyEntries {
                limit: self.entry_limit,
            });
        }
        self.entries += 1;
        let target_len = match &kind {
            Kind::Symlink(target) | Kind::HardLink(target) => {
                if target.as_os_str().is_empty()
                    || target.as_os_str().len() > 4096
                    || target.as_os_str().as_encoded_bytes().contains(&0)
                {
                    return Err(paths::violation(
                        &path,
                        "link target is empty, too long or contains NUL",
                    ));
                }
                target.as_os_str().len()
            }
            _ => 0,
        };
        if path.as_os_str().as_encoded_bytes().contains(&0) {
            return Err(paths::violation(&path, "name contains NUL"));
        }
        for parent in path
            .ancestors()
            .skip(1)
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            match self.nodes.get(parent) {
                Some(Kind::Directory { .. }) => {}
                Some(_) => {
                    return Err(paths::violation(
                        &path,
                        "entry is nested beneath a non-directory",
                    ));
                }
                None => {
                    self.charge(parent.as_os_str().len())?;
                    self.nodes
                        .insert(parent.to_owned(), Kind::Directory { explicit: false });
                }
            }
        }
        match self.nodes.get(&path) {
            Some(Kind::Directory { explicit: false }) if matches!(kind, Kind::Directory { .. }) => {
                *self.nodes.get_mut(&path).expect("parent exists") =
                    Kind::Directory { explicit: true };
            }
            Some(_) => {
                return Err(paths::violation(
                    &path,
                    "duplicate or conflicting payload name",
                ));
            }
            None => {
                self.charge(path.as_os_str().len() + target_len)?;
                self.nodes.insert(path, kind);
            }
        }
        Ok(())
    }

    /// Validate every link, not just names used by file writes. A returned
    /// root may never contain an escaping, cyclic or dangling link.
    pub(super) fn validate(&self) -> Result<(), FetchError> {
        for (name, kind) in &self.nodes {
            match kind {
                Kind::Symlink(_) => {
                    self.resolve_link(name, &mut BTreeSet::new(), &mut 0)?;
                }
                Kind::HardLink(target) => {
                    let target = paths::link_target(name, target, true)?;
                    if !matches!(self.nodes.get(&target), Some(Kind::Regular)) {
                        return Err(paths::violation(
                            name,
                            "hard link does not target a regular payload file",
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn symlink_target(&self, name: &Path) -> Option<&Path> {
        match self.nodes.get(name) {
            Some(Kind::Symlink(target)) => Some(target),
            _ => None,
        }
    }

    pub(super) fn entries(&self) -> impl Iterator<Item = (&Path, &Kind)> {
        self.nodes.iter().map(|(path, kind)| (path.as_path(), kind))
    }

    fn resolve_link(
        &self,
        name: &Path,
        visiting: &mut BTreeSet<PathBuf>,
        hops: &mut usize,
    ) -> Result<PathBuf, FetchError> {
        if *hops == MAX_LINK_HOPS || !visiting.insert(name.to_owned()) {
            return Err(paths::violation(
                name,
                "link cycle or traversal bound exceeded",
            ));
        }
        *hops += 1;
        let Some(Kind::Symlink(target)) = self.nodes.get(name) else {
            return Err(paths::violation(name, "link is absent from payload graph"));
        };
        if target.is_absolute() {
            return Err(paths::violation(name, "link target is absolute"));
        }
        let mut resolved = name.parent().unwrap_or(Path::new("")).to_owned();
        let mut parts = target.components().peekable();
        while let Some(part) = parts.next() {
            match part {
                Component::Normal(value) => {
                    resolved.push(value);
                    match self.nodes.get(&resolved) {
                        Some(Kind::Symlink(_)) => {
                            resolved = self.resolve_link(&resolved, visiting, hops)?;
                        }
                        Some(_) => {}
                        None => {
                            return Err(paths::violation(
                                name,
                                "link target is not in the payload",
                            ));
                        }
                    }
                }
                Component::CurDir => {}
                Component::ParentDir if resolved.pop() => {}
                Component::ParentDir => {
                    return Err(paths::violation(name, "composed link escapes the payload"));
                }
                _ => return Err(paths::violation(name, "link target is absolute")),
            }
            if parts.peek().is_some()
                && !resolved.as_os_str().is_empty()
                && !matches!(self.nodes.get(&resolved), Some(Kind::Directory { .. }))
            {
                return Err(paths::violation(name, "link crosses a non-directory"));
            }
        }
        if !resolved.as_os_str().is_empty() && !self.nodes.contains_key(&resolved) {
            return Err(paths::violation(name, "link target is not in the payload"));
        }
        visiting.remove(name);
        Ok(resolved)
    }
}
