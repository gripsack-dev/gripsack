//! Atomic filesystem primitives (0001 §9.2).
//!
//! Everything the store writes goes through here. The rules:
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

/// Write `contents` to `path` atomically: temp file in the same
/// directory, fsync, rename over, fsync the parent.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    io::Write::write_all(&mut tmp, contents)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    fsync_dir(parent)
}

/// Store pre-take-over bytes content-addressed (0015 §4): returns the
/// sha256 the manifest references. Dedup is the point — priors are
/// small, and identical originals share one blob.
pub fn store_prior_blob(home: &Path, bytes: &[u8]) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let sha = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let path = prior_blob_path(home, &sha);
    if !path.exists() {
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
    let parent = link.parent().unwrap_or_else(|| Path::new("."));
    let tmp = parent.join(format!(
        ".tmp-link-{}-{}",
        std::process::id(),
        link.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(target, &tmp).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("symlink {} -> {}: {e}", tmp.display(), target.display()),
        )
    })?;
    std::fs::rename(&tmp, link).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("link {} -> {}: {e}", link.display(), target.display()),
        )
    })?;
    fsync_dir(parent)
}

/// Publish a fully built directory into place. Fails if `dest` exists —
/// generations and store paths are immutable; publishing twice is a bug.
/// Payload FILES land read-only (0016 §D3): an app writing through an
/// owned symlink gets EACCES instead of silently corrupting the store.
/// Directories stay writable so repair/gc can unlink (unlink needs a
/// writable parent, not a writable file).
pub fn publish_dir(staging: &Path, dest: &Path) -> io::Result<()> {
    if dest.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "{} already exists — store paths are immutable",
                dest.display()
            ),
        ));
    }
    read_only_files(staging)?;
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    if let Err(e) = std::fs::rename(staging, dest) {
        // staging lives in $TMPDIR, the store under $GRIPSACK_HOME —
        // on containers /tmp is routinely a tmpfs, so EXDEV is a
        // layout fact, not a user error: fall back to a copy
        if e.kind() != io::ErrorKind::CrossesDevices {
            return Err(io::Error::new(
                e.kind(),
                format!("publish {} -> {}: {e}", staging.display(), dest.display()),
            ));
        }
        copy_dir(staging, dest)?;
        let _ = std::fs::remove_dir_all(staging);
    }
    fsync_dir(parent)
}

/// Recursively copy a directory tree: directories, regular files, and
/// symlinks (recreated, never followed). The destination is created —
/// or merged into — so a repo overlay can land on a fetched payload.
pub fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else if ty.is_symlink() {
            let target = std::fs::read_link(entry.path())?;
            std::os::unix::fs::symlink(target, &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// chmod every regular file under `dir` to drop write bits, keeping
/// exec (0016 §D3). Symlinks untouched (their target carries perms).
#[cfg(unix)]
fn read_only_files(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            read_only_files(&path)?;
        } else {
            let mode = meta.permissions().mode();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode & !0o222))?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn read_only_files(_dir: &Path) -> io::Result<()> {
    Ok(())
}

/// fsync a directory so renames into it are durable.
fn fsync_dir(dir: &Path) -> io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
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
