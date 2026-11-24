//! Read-only published store trees (issue #404).
//!
//! Publication already strips write bits from regular files (0016 §D3). Sealing
//! also strips them from directories, so software run out of the store (a
//! Python interpreter writing `__pycache__`) cannot add names to an immutable,
//! hash-addressed entry. Removal must first restore owner write access, which
//! [`remove_sealed_tree`] does for every maintenance path (gc, repair, rollback).
use super::{Dir, open_dir_nofollow};
use cap_std::fs::{Permissions, PermissionsExt};
use std::{io, path::Path};

/// Drop write bits from `name` and every directory below it, children first.
/// Symlinks and regular files are untouched.
pub fn seal_tree(parent: &Dir, name: &Path) -> io::Result<()> {
    let directory = open_dir_nofollow(parent, name)?;
    seal_children(&directory)?;
    set_mode(parent, name, |mode| mode & !0o222)
}

fn seal_children(directory: &Dir) -> io::Result<()> {
    for entry in directory.entries()? {
        let name = entry?.file_name();
        if directory.symlink_metadata(&name)?.is_dir() {
            let child = open_dir_nofollow(directory, Path::new(&name))?;
            seal_children(&child)?;
            set_mode(directory, Path::new(&name), |mode| mode & !0o222)?;
        }
    }
    Ok(())
}

/// Restore owner write access on `name` and every directory below it so the
/// tree can be unlinked. Files need no change: unlinking needs a writable parent.
pub fn unseal_tree(parent: &Dir, name: &Path) -> io::Result<()> {
    set_mode(parent, name, |mode| mode | 0o700)?;
    let directory = open_dir_nofollow(parent, name)?;
    unseal_children(&directory)
}

fn unseal_children(directory: &Dir) -> io::Result<()> {
    for entry in directory.entries()? {
        let name = entry?.file_name();
        if directory.symlink_metadata(&name)?.is_dir() {
            set_mode(directory, Path::new(&name), |mode| mode | 0o700)?;
            let child = open_dir_nofollow(directory, Path::new(&name))?;
            unseal_children(&child)?;
        }
    }
    Ok(())
}

/// Remove a possibly sealed directory tree.
pub fn remove_sealed_tree(parent: &Dir, name: &Path) -> io::Result<()> {
    match unseal_tree(parent, name) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    parent.remove_dir_all(name)
}

fn set_mode(parent: &Dir, name: &Path, change: impl Fn(u32) -> u32) -> io::Result<()> {
    let mode = parent.symlink_metadata(name)?.permissions().mode();
    parent.set_permissions(name, Permissions::from_mode(change(mode) & 0o7777))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn sealed_trees_refuse_new_names_and_are_removable() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("entry/lib/pkg")).unwrap();
        std::fs::write(root.path().join("entry/lib/pkg/mod.py"), b"x").unwrap();
        std::os::unix::fs::symlink("lib", root.path().join("entry/link")).unwrap();
        let home = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
        seal_tree(&home, Path::new("entry")).unwrap();
        let mode = |path: &str| {
            std::fs::metadata(root.path().join(path))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("entry"), 0o555);
        assert_eq!(mode("entry/lib/pkg"), 0o555);
        // Root bypasses mode bits; only assert the refusal for ordinary users.
        if rustix::process::geteuid().as_raw() != 0 {
            assert!(std::fs::create_dir(root.path().join("entry/lib/pkg/__pycache__")).is_err());
        }
        remove_sealed_tree(&home, Path::new("entry")).unwrap();
        assert!(!root.path().join("entry").exists());
        // Removing an absent tree is not an error.
        remove_sealed_tree(&home, Path::new("entry")).unwrap();
    }
}
