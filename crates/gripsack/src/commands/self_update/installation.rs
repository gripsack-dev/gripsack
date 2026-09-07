//! Pinned installation identity and capability-relative coordination/publication.

use cap_std::fs::MetadataExt as _;
use gripsack_fs::{Dir, cap_std};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

pub(super) fn current_target() -> io::Result<PathBuf> {
    normalize_target(std::env::current_exe()?)
}

fn normalize_target(path: PathBuf) -> io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    let path = {
        let mut path = path;
        use std::os::unix::ffi::OsStringExt;
        // A literal filename ending in this suffix is legal. Only a missing
        // original name permits interpreting it as the kernel's annotation.
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if let Some(bytes) = path.as_os_str().as_bytes().strip_suffix(b" (deleted)") {
                    path = PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()));
                }
            }
            Err(e) => return Err(e),
        }
        path
    };
    // Follow the install symlink once: publication replaces its target, not
    // the user-facing symlink. Missing targets fail closed.
    std::fs::canonicalize(path)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Snapshot {
    device: u64,
    inode: u64,
    length: u64,
    mode: u32,
    modified: (i64, i64),
    changed: (i64, i64),
}

pub(super) struct Installation {
    dir: Dir,
    parent: PathBuf,
    name: PathBuf,
    parent_identity: (u64, u64),
}

// Closing the descriptor releases flock on every return/unwind path. Never
// unlink a coordination file: waiters must continue locking the same inode.
pub(super) struct Lock {
    _file: cap_std::fs::File,
}

impl Installation {
    pub(super) fn pin(exe: &Path) -> io::Result<Self> {
        let parent = exe
            .parent()
            .ok_or_else(|| io::Error::other("executable has no parent"))?
            .to_owned();
        let name = PathBuf::from(
            exe.file_name()
                .ok_or_else(|| io::Error::other("executable has no name"))?,
        );
        let dir = gripsack_fs::open(&parent)?;
        let meta = dir.metadata(".")?;
        let result = Self {
            parent,
            name,
            parent_identity: (meta.dev(), meta.ino()),
            dir,
        };
        result.validate_parent()?;
        Ok(result)
    }

    fn validate_parent(&self) -> io::Result<()> {
        let meta = std::fs::metadata(&self.parent)?;
        if (meta.dev(), meta.ino()) != self.parent_identity
            || std::fs::canonicalize(&self.parent)? != self.parent
        {
            return Err(io::Error::other(
                "executable parent changed during self-update",
            ));
        }
        Ok(())
    }

    pub(super) fn lock(&self) -> io::Result<Lock> {
        self.validate_parent()?;
        let digest = gripsack_store::hash::hex_sha256(self.name.as_os_str().as_bytes());
        let name = format!(".grip-self-update-{digest}.flock");
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(false);
        // Use the existing workspace flock mechanism's syscall, but open the
        // coordination file relative to the pinned parent, not an ambient path.
        let file = self.dir.open_with(&name, &options)?;
        let opened = file.metadata()?;
        let named = self.dir.symlink_metadata(&name)?;
        if !named.is_file() || (opened.dev(), opened.ino()) != (named.dev(), named.ino()) {
            return Err(io::Error::other(
                "self-update lock is not a stable regular file",
            ));
        }
        loop {
            // SAFETY: file owns a live descriptor throughout this syscall and
            // the returned guard retains it for the complete critical section.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                break;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
        let lock = Lock { _file: file };
        self.validate_parent()?;
        let named = self.dir.symlink_metadata(&name)?;
        if !named.is_file() || (opened.dev(), opened.ino()) != (named.dev(), named.ino()) {
            return Err(io::Error::other("self-update lock changed while waiting"));
        }
        Ok(lock)
    }

    pub(super) fn snapshot(&self) -> io::Result<Snapshot> {
        self.validate_parent()?;
        let meta = self.dir.symlink_metadata(&self.name)?;
        if !meta.is_file() {
            return Err(io::Error::other(
                "installed executable is no longer a regular file",
            ));
        }
        Ok(Snapshot {
            device: meta.dev(),
            inode: meta.ino(),
            length: meta.len(),
            mode: meta.mode(),
            modified: (meta.mtime(), meta.mtime_nsec()),
            changed: (meta.ctime(), meta.ctime_nsec()),
        })
    }

    pub(super) fn require_snapshot(&self, expected: &Snapshot) -> io::Result<()> {
        if &self.snapshot()? != expected {
            return Err(io::Error::other(
                "installed executable changed outside the self-update lock; retry",
            ));
        }
        Ok(())
    }

    pub(super) fn publish(&self, source: &mut impl Read) -> io::Result<()> {
        self.validate_parent()?;
        gripsack_fs::atomic_copy_with_mode(&self.dir, &self.name, source, 0o755)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symlink_install_keeps_target_semantics() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("real");
        std::fs::write(&target, b"old").unwrap();
        let link = root.path().join("grip");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let install = Installation::pin(&normalize_target(link.clone()).unwrap()).unwrap();
        let _lock = install.lock().unwrap();
        install.publish(&mut &b"new"[..]).unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(link).unwrap(), b"new");
    }

    #[test]
    fn parent_drift_does_not_redirect_publication() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("bin");
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(parent.join("grip"), b"old").unwrap();
        let install =
            Installation::pin(&std::fs::canonicalize(parent.join("grip")).unwrap()).unwrap();
        std::fs::rename(&parent, root.path().join("moved")).unwrap();
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(parent.join("grip"), b"unrelated").unwrap();
        assert!(install.publish(&mut &b"new"[..]).is_err());
        assert_eq!(std::fs::read(parent.join("grip")).unwrap(), b"unrelated");
        assert_eq!(
            std::fs::read(root.path().join("moved/grip")).unwrap(),
            b"old"
        );
    }

    #[test]
    fn replacement_invalidates_installed_version_evidence() {
        let root = tempfile::tempdir().unwrap();
        let exe = root.path().join("grip");
        std::fs::write(&exe, b"old").unwrap();
        let install = Installation::pin(&std::fs::canonicalize(&exe).unwrap()).unwrap();
        let snapshot = install.snapshot().unwrap();
        std::fs::write(root.path().join("replacement"), b"new").unwrap();
        std::fs::rename(root.path().join("replacement"), exe).unwrap();
        assert!(install.require_snapshot(&snapshot).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn deleted_suffix_is_only_stripped_for_a_missing_original() {
        let root = tempfile::tempdir().unwrap();
        let normal = root.path().join("grip");
        let suffix = root.path().join("grip (deleted)");
        std::fs::write(&normal, b"installed").unwrap();
        assert_eq!(
            normalize_target(suffix.clone()).unwrap(),
            std::fs::canonicalize(&normal).unwrap()
        );
        std::fs::write(&suffix, b"literal filename").unwrap();
        assert_eq!(
            normalize_target(suffix.clone()).unwrap(),
            std::fs::canonicalize(suffix).unwrap()
        );
    }
}
