//! Canonical content identity (0008 §2): file type + executable bit +
//! contents, or symlink target. Mode bits beyond exec, mtimes, and
//! ownership are normalized away — so a fresh clone or a different umask
//! never changes identity, but `chmod +x` always does.

use sha2::{Digest, Sha256};
use std::path::Path;

/// Canonical hash of a single file system entry (file, dir, or symlink).
pub fn canonical_file_hash(path: &Path) -> std::io::Result<String> {
    let meta = std::fs::symlink_metadata(path)?;
    let mut hasher = Sha256::new();
    if meta.file_type().is_symlink() {
        hasher.update(b"link\0");
        hasher.update(std::fs::read_link(path)?.as_os_str().as_encoded_bytes());
    } else if meta.is_dir() {
        hasher.update(b"dir\0");
    } else {
        hasher.update(b"file\0");
        hasher.update([exec_byte(&meta)]);
        hasher.update(std::fs::read(path)?);
    }
    Ok(hex(&hasher.finalize()))
}

/// Canonical hash of in-memory file contents (no executable bit) — for
/// rendered templates and managed blocks, which have no store file.
pub fn canonical_bytes_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"file\0");
    hasher.update([0u8]);
    hasher.update(bytes);
    hex(&hasher.finalize())
}

/// Canonical hash of a directory tree: sorted relative paths plus each
/// entry's canonical identity. Deterministic across machines.
pub fn canonical_tree_hash(root: &Path) -> std::io::Result<String> {
    let mut entries = Vec::new();
    collect_entries(root, root, &mut entries)?;
    entries.sort();
    let mut hasher = Sha256::new();
    for rel in entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(canonical_file_hash(&root.join(&rel))?.as_bytes());
        hasher.update(b"\0");
    }
    Ok(hex(&hasher.finalize()))
}

fn collect_entries(root: &Path, dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let rel = path
            .strip_prefix(root)
            .expect("walked path is under root")
            .to_string_lossy()
            .into_owned();
        let is_real_dir = path.is_dir() && !path.is_symlink();
        out.push(rel);
        if is_real_dir {
            collect_entries(root, &path, out)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn exec_byte(meta: &std::fs::Metadata) -> u8 {
    use std::os::unix::fs::PermissionsExt;
    u8::from(meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn exec_byte(_meta: &std::fs::Metadata) -> u8 {
    1
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_hash_ignores_mtime_but_not_exec_bit() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tool");
        std::fs::write(&file, b"#!/bin/sh\necho hi\n").unwrap();
        let before = canonical_file_hash(&file).unwrap();
        // rewrite identical contents: mtime change must not change identity
        let contents = std::fs::read(&file).unwrap();
        std::fs::write(&file, contents).unwrap();
        assert_eq!(canonical_file_hash(&file).unwrap(), before);
        // chmod +x: must change identity
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(canonical_file_hash(&file).unwrap(), before);
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_ne!(canonical_file_hash(&file).unwrap(), before);
        }
    }

    #[test]
    fn canonical_hash_covers_symlink_target() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::write(dir.path().join("t1"), b"1").unwrap();
        std::fs::write(dir.path().join("t2"), b"2").unwrap();
        std::os::unix::fs::symlink("t1", &a).unwrap();
        std::os::unix::fs::symlink("t2", &b).unwrap();
        assert_ne!(
            canonical_file_hash(&a).unwrap(),
            canonical_file_hash(&b).unwrap()
        );
    }

    #[test]
    fn tree_hash_is_deterministic_and_content_sensitive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/x"), b"x").unwrap();
        std::fs::write(dir.path().join("y"), b"y").unwrap();
        let before = canonical_tree_hash(dir.path()).unwrap();
        assert_eq!(canonical_tree_hash(dir.path()).unwrap(), before);
        std::fs::write(dir.path().join("z"), b"z").unwrap();
        assert_ne!(canonical_tree_hash(dir.path()).unwrap(), before);
    }
}

#[cfg(test)]
mod reference_vector {
    //! The cross-implementation test vector (0012): this exact tree
    //! must hash to this exact value in EVERY implementation — the
    //! conformance suite and plugins mirror it. If you change the
    //! algorithm, all mirrors change with it (don't).

    #[test]
    fn pinned_tree_vector() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("share")).unwrap();
        std::fs::write(root.join("bin/hello"), b"#!/bin/sh\necho hello\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                root.join("bin/hello"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
            std::os::unix::fs::symlink("hello", root.join("bin/hi")).unwrap();
        }
        std::fs::write(root.join("share/version.txt"), b"1.0\n").unwrap();
        let hash = crate::canonical_tree_hash(root).unwrap();
        assert_eq!(
            hash,
            "cce3e9f819b476cc5abed85b83f2f1a01cac2abd4c2eb34f08b76d822739e595"
        );
    }
}
