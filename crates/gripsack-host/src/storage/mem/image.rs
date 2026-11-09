//! One filesystem image: an inode table. `MemFs` keeps two — the volatile
//! image every operation sees and the durable image a power loss starts
//! from — plus the updates in between.
use crate::storage::{Name, Permissions};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub(super) type Ino = u64;
pub(super) const ROOT: Ino = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Node {
    File {
        data: Vec<u8>,
        permissions: Permissions,
    },
    Dir {
        entries: BTreeMap<Name, Ino>,
        permissions: Permissions,
        /// rmdir'd in this image: no new entries may be created in it.
        removed: bool,
    },
    /// The target is written with the inode (a fast symlink), so it is never
    /// torn; only the entry naming it needs a barrier.
    Symlink { target: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Image {
    pub(super) nodes: BTreeMap<Ino, Node>,
}

impl Image {
    pub(super) fn empty() -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            ROOT,
            Node::Dir {
                entries: BTreeMap::new(),
                permissions: Permissions::new(0o755),
                removed: false,
            },
        );
        Self { nodes }
    }

    pub(super) fn node(&self, ino: Ino) -> Option<&Node> {
        self.nodes.get(&ino)
    }

    pub(super) fn entries(&self, dir: Ino) -> Option<&BTreeMap<Name, Ino>> {
        match self.nodes.get(&dir) {
            Some(Node::Dir { entries, .. }) => Some(entries),
            _ => None,
        }
    }

    pub(super) fn lookup(&self, dir: Ino, name: &Name) -> Option<Ino> {
        self.entries(dir)
            .and_then(|entries| entries.get(name).copied())
    }

    fn entries_mut(&mut self, dir: Ino) -> &mut BTreeMap<Name, Ino> {
        match self.nodes.get_mut(&dir) {
            Some(Node::Dir { entries, .. }) => entries,
            _ => unreachable!("updates name only directory inodes as parents"),
        }
    }
}

/// One update that reaches the durable image only through a barrier (S2)
/// or by surviving a power loss (S3). Updates are idempotent state
/// assignments, so any subset applies in trace order without failing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Update {
    /// Set (`Some`) or remove (`None`) one directory entry.
    Entry {
        dir: Ino,
        name: Name,
        target: Option<Ino>,
    },
    /// Rename within one directory: one update, so it is atomic (S1).
    Rename {
        dir: Ino,
        from: Name,
        to: Name,
        ino: Ino,
    },
    /// Bytes at an offset; a gap reads as zeros (a torn write).
    Data {
        ino: Ino,
        offset: u64,
        bytes: Vec<u8>,
    },
    Mode {
        ino: Ino,
        permissions: Permissions,
    },
}

/// What a barrier covers (S2).
#[derive(Debug, Clone, Copy)]
pub(super) enum Barrier {
    /// `fsync(file)`: the inode's bytes and mode.
    File(Ino),
    /// `fsync(dir)`: the directory's entries.
    Dir(Ino),
}

impl Update {
    pub(super) fn covered_by(&self, barrier: Barrier) -> bool {
        match (self, barrier) {
            (Update::Data { ino, .. } | Update::Mode { ino, .. }, Barrier::File(file)) => {
                *ino == file
            }
            (Update::Entry { dir, .. } | Update::Rename { dir, .. }, Barrier::Dir(barrier)) => {
                *dir == barrier
            }
            _ => false,
        }
    }

    pub(super) fn apply(&self, image: &mut Image) {
        match self {
            Update::Entry { dir, name, target } => {
                let entries = image.entries_mut(*dir);
                match target {
                    Some(ino) => {
                        entries.insert(name.clone(), *ino);
                    }
                    None => {
                        entries.remove(name);
                    }
                }
            }
            Update::Rename { dir, from, to, ino } => {
                let entries = image.entries_mut(*dir);
                entries.insert(to.clone(), *ino);
                if entries.get(from) == Some(ino) {
                    entries.remove(from);
                }
            }
            Update::Data { ino, offset, bytes } => {
                if let Some(Node::File { data, .. }) = image.nodes.get_mut(ino) {
                    let start = usize::try_from(*offset).expect("in-memory offsets fit usize");
                    let end = start + bytes.len();
                    if data.len() < end {
                        data.resize(end, 0);
                    }
                    data[start..end].copy_from_slice(bytes);
                }
            }
            Update::Mode { ino, permissions } => {
                if let Some(Node::File { permissions: p, .. } | Node::Dir { permissions: p, .. }) =
                    image.nodes.get_mut(ino)
                {
                    *p = *permissions;
                }
            }
        }
    }
}
