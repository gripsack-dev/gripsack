//! Logical segment-prefix selection, independent of physical alias resolution.
use gripsack_ir::workspace::WorkspaceDestination;
use std::io;

pub(super) const MAX_EXPANDED_FILES: usize = 10_000;
// Match the captured source inventory's depth policy; alias expansion must not
// create an unbounded logical tree from a bounded physical inventory.
pub(super) const MAX_TREE_DEPTH: usize = 128;

pub(super) struct Selection<'a> {
    include: &'a [String],
    exclude: &'a [String],
}

impl<'a> Selection<'a> {
    pub fn new(include: &'a [String], exclude: &'a [String]) -> Self {
        Self { include, exclude }
    }

    pub fn visits(&self, path: &str) -> bool {
        !self.exclude.iter().any(|prefix| covers(prefix, path))
            && (self.include.is_empty()
                || self
                    .include
                    .iter()
                    .any(|prefix| covers(prefix, path) || covers(path, prefix)))
    }

    pub fn selects(&self, path: &str) -> bool {
        !self.exclude.iter().any(|prefix| covers(prefix, path))
            && (self.include.is_empty() || self.include.iter().any(|prefix| covers(prefix, path)))
    }
}

fn covers(prefix: &str, path: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('/'))
}

pub(super) fn destination(root: &WorkspaceDestination, relative: &str) -> WorkspaceDestination {
    let join = |root: &str| format!("{}/{relative}", root.trim_end_matches('/'));
    match root {
        WorkspaceDestination::Symlink { path } => {
            WorkspaceDestination::Symlink { path: join(path) }
        }
        WorkspaceDestination::TrackedCopy { path } => {
            WorkspaceDestination::TrackedCopy { path: join(path) }
        }
        WorkspaceDestination::ManagedBlock { path, marker } => WorkspaceDestination::ManagedBlock {
            path: join(path),
            marker: marker.clone(),
        },
    }
}

pub(super) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
