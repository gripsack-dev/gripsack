//! Destination identity for the apply machine (plan/0056 §4 S6, FND-3).
//!
//! The kernel compares destinations only as interned [`DestId`]s, so the
//! interning is where the filesystem's name equivalence enters: two spellings
//! are one destination when they are byte-equal, or when they differ only in
//! case (the fold destination admission uses) and the filesystem resolves
//! both to the same object right now. On a case-insensitive mount (WSL's
//! `/mnt/c`, macOS by default) re-spelling a destination's case therefore
//! keeps its identity: the previous generation's record follows it, and the
//! old spelling is not released under the new one's feet. On a case-sensitive
//! filesystem the two spellings are distinct objects and stay distinct (a
//! hard link between them is judged one object: the conservative side, which
//! never releases it).
use gripsack_kernel::DestId;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The admission fold (`deploy/destinations.rs`): one key per case class.
fn fold(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// The object a path names, without following a final symlink.
#[cfg(unix)]
fn object(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path)
        .ok()
        .map(|metadata| (metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn object(_path: &Path) -> Option<(u64, u64)> {
    None
}

/// Whether two destination spellings name one object on this filesystem:
/// byte-equal, or equal under the admission fold and resolving to the same
/// object now.
pub(crate) fn same_destination(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if fold(a) != fold(b) {
        return false;
    }
    match (object(a), object(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// The run's destination table.
#[derive(Default)]
pub(crate) struct Dests {
    paths: Vec<PathBuf>,
    exact: BTreeMap<PathBuf, u32>,
    folded: BTreeMap<String, Vec<u32>>,
}

impl Dests {
    /// The destination's identity, probing the filesystem now: a spelling the
    /// filesystem resolves to an already interned destination's object is
    /// that destination.
    pub(crate) fn intern(&mut self, path: &Path) -> std::io::Result<DestId> {
        if let Some(index) = self.exact.get(path) {
            return Ok(DestId::new(*index));
        }
        let key = fold(path);
        if let Some(candidates) = self.folded.get(&key) {
            for index in candidates {
                if same_destination(&self.paths[*index as usize], path) {
                    return Ok(DestId::new(*index));
                }
            }
        }
        let index = u32::try_from(self.paths.len())
            .map_err(|_| std::io::Error::other("too many destinations in one apply"))?;
        self.paths.push(path.to_owned());
        self.exact.insert(path.to_owned(), index);
        self.folded.entry(key).or_default().push(index);
        Ok(DestId::new(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_are_one_destination_only_when_the_filesystem_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let upper = dir.path().join("Config.toml");
        let lower = dir.path().join("config.toml");
        std::fs::write(&upper, "a").unwrap();
        let mut dests = Dests::default();
        let first = dests.intern(&upper).unwrap();
        assert_eq!(dests.intern(&upper).unwrap(), first);
        // case-sensitive here: the other spelling is a different object
        let other = dests.intern(&lower).unwrap();
        assert_ne!(other, first);
        assert!(!same_destination(&upper, &lower));
        // a filesystem that resolves both spellings to one object (a
        // case-insensitive mount; here a hard link stands in for it)
        let dir = tempfile::tempdir().unwrap();
        let upper = dir.path().join("Config.toml");
        let lower = dir.path().join("config.toml");
        std::fs::write(&upper, "a").unwrap();
        std::fs::hard_link(&upper, &lower).unwrap();
        let mut dests = Dests::default();
        let first = dests.intern(&upper).unwrap();
        assert_eq!(dests.intern(&lower).unwrap(), first);
        // the fold is required: an unrelated hard link stays distinct
        let elsewhere = dir.path().join("elsewhere");
        std::fs::hard_link(&upper, &elsewhere).unwrap();
        assert_ne!(dests.intern(&elsewhere).unwrap(), first);
    }
}
