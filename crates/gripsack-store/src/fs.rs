//! Atomic filesystem primitives (0001 §9.2) — MIGRATION WRAPPERS
//! (plan/0021).
//!
//! The mechanics now live in `gripsack-fs`, capability-based. These
//! string-path wrappers exist so callers migrate phase by phase;
//! phase 5 deletes this module entirely and callers use `gripsack-fs`
//! directly. The rules the wrappers preserve:
//!
//! - writes are staged in the same directory and renamed into place —
//!   a reader never sees a partial file;
//! - symlink swaps rename over a temp link — a generation flip is
//!   indivisible (`current` is either the old generation or the new
//!   one, never missing);
//! - file and parent dir are fsync'd before the call returns, so a
//!   crash mid-apply cannot lose the rename.

use std::io;
use std::path::Path;

pub use gripsack_fs::FlockGuard;

/// Write `contents` to `path` atomically: temp file in the same
/// directory, fsync, rename over, fsync the parent.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    gripsack_fs::atomic_write_at(path, contents)
}

/// Store pre-take-over bytes content-addressed (0015 §4): returns the
/// sha256 the manifest references. Dedup is the point — priors are
/// small, and identical originals share one blob.
pub fn store_prior_blob(home: &Path, bytes: &[u8]) -> io::Result<String> {
    let sha = crate::hash::hex_sha256(bytes);
    let path = prior_blob_path(home, &sha);
    // exists() follows symlinks: a planted `prior/<sha>` symlink would
    // skip the write and later restore THROUGH it. Skip only a real
    // regular file; anything else is replaced by the atomic rename.
    let present = std::fs::symlink_metadata(&path)
        .map(|m| m.is_file())
        .unwrap_or(false);
    if !present {
        atomic_write(&path, bytes)?;
    }
    Ok(sha)
}

/// Where a prior blob lives: `$GRIPSACK_HOME/prior/<sha256>`.
pub fn prior_blob_path(home: &Path, sha: &str) -> std::path::PathBuf {
    home.join("prior").join(sha)
}

/// Atomically point `link` at `target`, replacing any existing link.
/// This is the generation flip — the single indivisible operation that
/// activation reduces to (0001 §9.2).
pub fn symlink_replace(link: &Path, target: &Path) -> io::Result<()> {
    gripsack_fs::symlink_replace_at(link, target)
}

/// Publish a fully built directory into place. Fails if `dest` exists —
/// generations and store paths are immutable; publishing twice is a bug.
/// Payload FILES land read-only (0016 §D3): an app writing through an
/// owned symlink gets EACCES instead of silently corrupting the store.
/// Directories stay writable so repair/gc can unlink (unlink needs a
/// writable parent, not a writable file).
pub fn publish_dir(staging: &Path, dest: &Path) -> io::Result<()> {
    gripsack_fs::publish_dir_at(staging, dest)
}

/// Recursively copy a directory tree: directories, regular files, and
/// symlinks (recreated, never followed). The destination is created —
/// or merged into — so a repo overlay can land on a fetched payload.
pub fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    gripsack_fs::copy_dir(src, dst)
}

/// fsync a directory so renames into it are durable.
pub(crate) fn fsync_dir(dir: &Path) -> io::Result<()> {
    let cap = gripsack_fs::open(dir)?;
    gripsack_fs::fsync_dir(&cap, Path::new("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_lands_content_without_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("manifest.json");
        atomic_write(&file, br#"{"gen": 1}"#).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), br#"{"gen": 1}"#);
        // overwrite works, and no temp files linger
        atomic_write(&file, br#"{"gen": 2}"#).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), br#"{"gen": 2}"#);
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn symlink_replace_swaps_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let (gen1, gen2) = (
            dir.path().join("generations/1"),
            dir.path().join("generations/2"),
        );
        std::fs::create_dir_all(&gen1).unwrap();
        std::fs::create_dir_all(&gen2).unwrap();
        let current = dir.path().join("current");
        symlink_replace(&current, &gen1).unwrap();
        assert_eq!(current.read_link().unwrap(), gen1);
        // the flip: current is never absent between the two states
        symlink_replace(&current, &gen2).unwrap();
        assert_eq!(current.read_link().unwrap(), gen2);
    }

    #[test]
    fn publish_dir_moves_and_refuses_republish() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("payload"), b"bytes").unwrap();
        let dest = dir.path().join("store/abc123-helix");
        publish_dir(&staging, &dest).unwrap();
        assert!(dest.join("payload").exists());
        assert!(!staging.exists());
        // immutable: a second publish into the same path errors
        let staging2 = dir.path().join("staging2");
        std::fs::create_dir_all(&staging2).unwrap();
        let err = publish_dir(&staging2, &dest).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn copy_dir_preserves_symlinks_and_merges() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/f"), b"bytes").unwrap();
        std::os::unix::fs::symlink("f", src.join("sub/link")).unwrap();
        // merge: the destination already holds a fetched payload dir
        let dst = dir.path().join("dst");
        std::fs::create_dir_all(dst.join("sub")).unwrap();
        std::fs::write(dst.join("sub/payload"), b"fetched").unwrap();
        copy_dir(&src, &dst).unwrap();
        assert_eq!(std::fs::read(dst.join("sub/f")).unwrap(), b"bytes");
        assert_eq!(std::fs::read(dst.join("sub/payload")).unwrap(), b"fetched");
        let link = dst.join("sub/link");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_link(link).unwrap(), Path::new("f"));
    }
}
