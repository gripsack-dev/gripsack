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
    } else if meta.is_file() {
        hasher.update(b"file\0");
        hasher.update([exec_byte(&meta)]);
        hasher.update(std::fs::read(path)?);
    } else {
        // fifos, sockets, device nodes: reading one blocks forever (or
        // OOMs on /dev/zero) — a store entry is a regular file
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "{} is not a regular file, directory, or symlink",
                path.display()
            ),
        ));
    }
    Ok(hex(&hasher.finalize()))
}

/// [`canonical_file_hash`] through a directory capability
/// (plan/0021): deploy's drift check hashes the destination relative
/// to the SAME pinned parent inode the subsequent write uses, so the
/// check and the use cannot observe different filesystems.
pub fn canonical_file_hash_in(dir: &gripsack_fs::Dir, name: &Path) -> std::io::Result<String> {
    let meta = dir.symlink_metadata(name)?;
    let mut hasher = Sha256::new();
    if meta.file_type().is_symlink() {
        hasher.update(b"link\0");
        hasher.update(dir.read_link_contents(name)?.as_os_str().as_encoded_bytes());
    } else if meta.is_dir() {
        hasher.update(b"dir\0");
    } else if meta.is_file() {
        hasher.update(b"file\0");
        hasher.update([exec_byte_in(&meta)]);
        hasher.update(dir.read(name)?);
    } else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{name:?} is not a regular file, directory, or symlink"),
        ));
    }
    Ok(hex(&hasher.finalize()))
}

/// Canonical identity of in-memory contents WITH their permission
/// mode (0026 §7, 0030 #17, plan/0031): the journal's and the
/// manifest's file identity is mode-aware, so a chmod-only change is
/// drift, never invisible. Distinct preimage tag from
/// [`canonical_bytes_hash`] — the bytes-only (mode-unmanaged) and
/// mode-aware domains never compare equal by accident.
pub fn canonical_bytes_identity(bytes: &[u8], mode: u32) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"file\0");
    hasher.update([1u8]);
    hasher.update(mode.to_le_bytes());
    hasher.update(bytes);
    hex(&hasher.finalize())
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

/// Canonical tree hash of a NOT-YET-STAGED overlay (0014): repo files
/// at their `from` relative paths, ancestor dirs synthesized — the same
/// digest `canonical_tree_hash` gives the staged directory, without
/// materializing it. Plan-time content identity for config-only
/// modules.
pub fn canonical_overlay_hash(repo: &Path, froms: &[String]) -> std::io::Result<String> {
    let dir_hash = hex(&Sha256::digest(b"dir\0"));
    let mut entries: Vec<(String, String)> = Vec::new();
    for from in froms {
        let source = repo.join(from);
        if source.is_dir() && !source.symlink_metadata()?.file_type().is_symlink() {
            // a directory `from` stages recursively at publish — hash
            // the same closure here or plan-time and publish-time
            // identity diverge
            let mut ancestor = Path::new(from.as_str()).parent();
            while let Some(dir) = ancestor {
                if dir.as_os_str().is_empty() {
                    break;
                }
                entries.push((dir.to_string_lossy().into_owned(), dir_hash.clone()));
                ancestor = dir.parent();
            }
            entries.push((from.clone(), dir_hash.clone()));
            let mut rels = Vec::new();
            collect_entries(&source, &source, &mut rels)?;
            for rel in rels {
                let rel_path = format!("{from}/{rel}");
                let hash = if source.join(&rel).is_dir() && !source.join(&rel).is_symlink() {
                    dir_hash.clone()
                } else {
                    canonical_file_hash(&source.join(&rel))?
                };
                entries.push((rel_path, hash));
            }
            continue;
        }
        if !source.is_file() {
            continue;
        }
        // ancestor dirs are stage entries too (create_dir_all at
        // publish) — synthesize them or the digests diverge
        let mut ancestor = Path::new(from.as_str()).parent();
        while let Some(dir) = ancestor {
            if dir.as_os_str().is_empty() {
                break;
            }
            let rel = dir.to_string_lossy().into_owned();
            entries.push((rel, dir_hash.clone()));
            ancestor = dir.parent();
        }
        entries.push((from.clone(), canonical_file_hash(&source)?));
    }
    entries.sort();
    entries.dedup();
    let mut hasher = Sha256::new();
    for (rel, hash) in entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(hash.as_bytes());
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

#[cfg(unix)]
fn exec_byte_in(meta: &gripsack_fs::cap_std::fs::Metadata) -> u8 {
    use gripsack_fs::cap_std::fs::MetadataExt;
    u8::from(meta.mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn exec_byte_in(_meta: &gripsack_fs::cap_std::fs::Metadata) -> u8 {
    1
}

#[cfg(not(unix))]
fn exec_byte(_meta: &std::fs::Metadata) -> u8 {
    1
}

fn hex(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for &b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// sha256 of raw bytes as lowercase hex — the one true encoder for
/// digests in this crate (paths and blobs used to roll their own).
pub fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_two_identity_domains_never_collide() {
        // bytes-only (templates: mode unmanaged) vs mode-aware
        // (tracked copies, the journal): same bytes, same mode —
        // different identities, so a domain mix-up can never read as
        // "satisfied"
        assert_ne!(
            canonical_bytes_hash(b"abc"),
            canonical_bytes_identity(b"abc", 0o644),
        );
        // the mode is IN the identity, not just the exec bit
        assert_ne!(
            canonical_bytes_identity(b"abc", 0o600),
            canonical_bytes_identity(b"abc", 0o644),
        );
        assert_ne!(
            canonical_bytes_identity(b"abc", 0o600),
            canonical_bytes_identity(b"abc", 0o700),
        );
    }

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
    fn overlay_hash_matches_materialized_staging() {
        // 0014's load-bearing invariant: the plan-time overlay hash of
        // repo sources equals the tree hash of the staging publish
        // would assemble — ancestor dirs synthesized, exec bit covered
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join("configs/demo")).unwrap();
        std::fs::write(repo.join("configs/demo/a.toml"), b"a\n").unwrap();
        std::fs::write(repo.join("tool.sh"), b"#!/bin/sh\necho hi\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(repo.join("tool.sh"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let froms = vec!["configs/demo/a.toml".to_string(), "tool.sh".to_string()];
        let overlay = canonical_overlay_hash(&repo, &froms).unwrap();

        let stage = dir.path().join("stage");
        for from in &froms {
            let dest = stage.join(from);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::copy(repo.join(from), dest).unwrap();
        }
        assert_eq!(overlay, canonical_tree_hash(&stage).unwrap());

        // a content edit moves the overlay hash
        std::fs::write(repo.join("configs/demo/a.toml"), b"b\n").unwrap();
        assert_ne!(canonical_overlay_hash(&repo, &froms).unwrap(), overlay);
    }

    #[test]
    fn overlay_hash_of_dir_source_matches_staged_tree() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join("configs/app/nested")).unwrap();
        std::fs::write(repo.join("configs/app/a.toml"), b"a\n").unwrap();
        std::fs::write(repo.join("configs/app/nested/b.toml"), b"b\n").unwrap();
        std::os::unix::fs::symlink("a.toml", repo.join("configs/app/link")).unwrap();
        let froms = vec!["configs/app".to_string()];
        let overlay = canonical_overlay_hash(&repo, &froms).unwrap();
        // what publish stages: the dir copied under its from path
        let staged = dir.path().join("staged");
        gripsack_fs::copy_dir(&repo.join("configs/app"), &staged.join("configs/app")).unwrap();
        assert_eq!(
            overlay,
            canonical_tree_hash(&staged).unwrap(),
            "plan-time overlay and staged tree must agree or the pinned tree check false-fires"
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
