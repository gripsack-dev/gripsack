//! Input-addressed store paths (plan/0001 §3.4).
//!
//! A store path is `<gripsack-home>/store/<input-hash>-<name>` where the
//! input hash covers the resolved module plan (source + pinned refs +
//! build recipe + dependency hashes). Store paths are immutable; the same
//! resolved inputs produce the same path on any machine of the same
//! platform — that is what makes store sharing a trivial later feature.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const STORE_DIR: &str = "store";
pub const GENERATIONS_DIR: &str = "generations";
/// Length of the hex input hash prefix in store path names.
pub const HASH_LEN: usize = 16;

/// Hex sha256 prefix of the canonical serialized module plan. `canonical`
/// must be a stable serialization (serde struct field order is).
pub fn input_hash(canonical: &str) -> String {
    let digest = Sha256::digest(canonical.as_bytes());
    digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .take(HASH_LEN)
        .collect()
}

/// The store path for a module with the given resolved plan.
pub fn store_path(home: &Path, name: &str, canonical: &str) -> PathBuf {
    home.join(STORE_DIR)
        .join(format!("{}-{name}", input_hash(canonical)))
}

/// Base directory for everything gripsack owns: store, generations, and
/// the `current` symlink. `$GRIPSACK_HOME` wins, then
/// `$XDG_DATA_HOME/gripsack`, then `~/.local/share/gripsack`.
pub fn gripsack_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("GRIPSACK_HOME") {
        return PathBuf::from(dir);
    }
    if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(data).join("gripsack");
    }
    let home = std::env::var_os("HOME").expect("HOME is not set");
    PathBuf::from(home).join(".local/share/gripsack")
}

// ---------------------------------------------------------------- canonical hashing

/// Canonical content identity (0008 §2): file type + executable bit +
/// contents, or symlink target. Mode bits beyond exec, mtimes, and
/// ownership are normalized away — so a fresh clone or a different umask
/// never changes identity, but `chmod +x` always does.
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

/// The `current` symlink — flipping it IS activation (0001 §9.2).
pub fn current_link(home: &Path) -> PathBuf {
    home.join("current")
}

/// Directory of one generation's profile tree.
pub fn generation_dir(home: &Path, generation: u64) -> PathBuf {
    home.join(GENERATIONS_DIR).join(generation.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_input_sensitive() {
        assert_eq!(input_hash("plan-a"), input_hash("plan-a"));
        assert_ne!(input_hash("plan-a"), input_hash("plan-b"));
        assert_eq!(input_hash("plan-a").len(), HASH_LEN * 2);
    }

    #[test]
    fn store_path_format() {
        let p = store_path(Path::new("/gs"), "helix", "plan-a");
        let name = p.file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with("-helix"));
        assert_eq!(name.len(), HASH_LEN * 2 + 1 + "helix".len());
        assert_eq!(p.parent().unwrap(), Path::new("/gs/store"));
    }

    #[test]
    fn home_resolution() {
        // Single test mutating env to avoid cross-test races.
        std::env::set_var("GRIPSACK_HOME", "/tmp/gs-explicit");
        assert_eq!(gripsack_home(), Path::new("/tmp/gs-explicit"));
        std::env::remove_var("GRIPSACK_HOME");
        std::env::set_var("XDG_DATA_HOME", "/tmp/gs-xdg");
        assert_eq!(gripsack_home(), Path::new("/tmp/gs-xdg/gripsack"));
        std::env::remove_var("XDG_DATA_HOME");
        std::env::set_var("HOME", "/tmp/gs-home");
        assert_eq!(
            gripsack_home(),
            Path::new("/tmp/gs-home/.local/share/gripsack")
        );
    }

    #[test]
    fn canonical_hash_ignores_mtime_but_not_exec_bit() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tool");
        std::fs::write(&file, b"#!/bin/sh\necho hi\n").unwrap();
        let before = canonical_file_hash(&file).unwrap();
        // touch: mtime change must not change identity
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

    #[test]
    fn generation_layout() {
        let home = Path::new("/gs");
        assert_eq!(current_link(home), Path::new("/gs/current"));
        assert_eq!(generation_dir(home, 42), Path::new("/gs/generations/42"));
    }
}
