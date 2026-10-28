//! Canonical content identity (0008 §2): file type + executable bit +
//! contents, or symlink target. Mode bits beyond exec, mtimes, and
//! ownership are normalized away — so a fresh clone or a different umask
//! never changes identity, but `chmod +x` always does.

use sha2::{Digest, Sha256};
use std::path::Path;

/// The hash DOMAINS, typed: gripsack hashes content for three
/// different purposes and a bare `String` lets one domain's hash
/// silently compare against another's (a merge arm once checked a
/// bytes-only hash against the mode-aware journal identity and
/// aborted every merge write). Each newtype comes from exactly one
/// constructor family, so a cross-domain mix is a type error.
macro_rules! hash_domain {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<$name> for String {
            fn from(h: $name) -> String {
                h.0
            }
        }
    };
}

hash_domain!(
    PayloadHash,
    "A store payload's content identity (0008 §2): names the store path, \
     drives dedup and the lockfile pin. Exec-bit + bytes preimage."
);
hash_domain!(
    BytesHash,
    "Bytes-only content identity: merge blocks and symlink targets \
     observed at whole-file destinations. Never interchangeable with `FileIdentity`."
);
/// The manifest's deployed-entry identity (0029 §2, 0035): MODAL by
/// ownership mode — a copy/template's FileIdentity, a merge block's
/// BytesHash plus hosting mode, a link's PayloadHash. The wire is a plain
/// hex string (back-compat); the newtype's constructors make a raw
/// string unconstructible in production code, so the modal domains
/// can only ever be compared within their own mode's rows.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ManifestHash(String);

impl ManifestHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// For test fixtures and wire-boundary reassembly of a value a
    /// typed producer just computed (the Preserve record).
    #[doc(hidden)]
    pub fn from_raw(s: String) -> Self {
        ManifestHash(s)
    }
}

impl std::fmt::Display for ManifestHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

macro_rules! manifest_hash_from {
    ($t:ty) => {
        impl From<$t> for ManifestHash {
            fn from(h: $t) -> ManifestHash {
                ManifestHash(h.to_string())
            }
        }
    };
}
manifest_hash_from!(FileIdentity);
manifest_hash_from!(BytesHash);
manifest_hash_from!(PayloadHash);

hash_domain!(
    FileIdentity,
    "Mode-aware file identity (0031): bytes + full permission mode. The \
     journal's identity domain and the copy/template manifest domain. \
     Never interchangeable with `BytesHash`."
);
/// Canonical hash of a single file system entry (file, dir, or symlink).
pub fn canonical_file_hash(path: &Path) -> std::io::Result<PayloadHash> {
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
        append_contents(&mut hasher, std::fs::File::open(path)?)?;
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
    Ok(PayloadHash(hex(&hasher.finalize())))
}

/// [`canonical_file_hash`] through a directory capability
/// (plan/0021): deploy's drift check hashes the destination relative
/// to the SAME pinned parent inode the subsequent write uses, so the
/// check and the use cannot observe different filesystems.
pub fn canonical_file_hash_in(dir: &gripsack_fs::Dir, name: &Path) -> std::io::Result<PayloadHash> {
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
        append_contents(&mut hasher, dir.open(name)?)?;
    } else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{name:?} is not a regular file, directory, or symlink"),
        ));
    }
    Ok(PayloadHash(hex(&hasher.finalize())))
}

/// Hash files through a fixed buffer: tree identity must not reintroduce
/// a whole-payload allocation after bounded extraction.
fn append_contents(hasher: &mut Sha256, mut reader: impl std::io::Read) -> std::io::Result<()> {
    let mut buffer = [0; 64 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => hasher.update(&buffer[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

/// Canonical identity of in-memory contents WITH their permission
/// mode (0026 §7, 0030 #17, plan/0031): the journal's and the
/// manifest's file identity is mode-aware, so a chmod-only change is
/// drift, never invisible. Distinct preimage tag from
/// [`canonical_bytes_hash`] — the bytes-only (mode-unmanaged) and
/// mode-aware domains never compare equal by accident.
pub fn canonical_bytes_identity(bytes: &[u8], mode: u32) -> FileIdentity {
    let mut hasher = Sha256::new();
    hasher.update(b"file\0");
    hasher.update([1u8]);
    hasher.update(mode.to_le_bytes());
    hasher.update(bytes);
    FileIdentity(hex(&hasher.finalize()))
}

/// Canonical hash of in-memory content without permissions (managed blocks).
pub fn canonical_bytes_hash(bytes: &[u8]) -> BytesHash {
    let mut hasher = Sha256::new();
    hasher.update(b"file\0");
    hasher.update([0u8]);
    hasher.update(bytes);
    BytesHash(hex(&hasher.finalize()))
}

/// Canonical hash of a directory tree: sorted relative paths plus each
/// entry's canonical identity. Deterministic across machines.
pub fn canonical_tree_hash(root: &Path) -> std::io::Result<PayloadHash> {
    canonical_tree_hash_in(&gripsack_fs::open(root)?)
}

/// The canonical tree domain through an already pinned directory capability.
pub fn canonical_tree_hash_in(root: &gripsack_fs::Dir) -> std::io::Result<PayloadHash> {
    let mut entries = Vec::new();
    collect_entries(root, Path::new(""), &mut entries)?;
    entries.sort();
    let mut hasher = Sha256::new();
    for rel in entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(
            canonical_file_hash_in(root, Path::new(&rel))?
                .as_str()
                .as_bytes(),
        );
        hasher.update(b"\0");
    }
    Ok(PayloadHash(hex(&hasher.finalize())))
}

fn collect_entries(
    directory: &gripsack_fs::Dir,
    prefix: &Path,
    out: &mut Vec<String>,
) -> std::io::Result<()> {
    for entry in directory.entries()? {
        let name = entry?.file_name();
        let name = Path::new(&name);
        let relative = prefix.join(name);
        let metadata = directory.symlink_metadata(name)?;
        out.push(relative.to_string_lossy().into_owned());
        if metadata.is_dir() {
            let child = gripsack_fs::open_dir_nofollow(directory, name)?;
            collect_entries(&child, &relative, out)?;
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
        // (tracked copies, the journal): different TYPES — a domain
        // mix-up doesn't compile. Their wire forms differ too:
        assert_ne!(
            canonical_bytes_hash(b"abc").as_str(),
            canonical_bytes_identity(b"abc", 0o644).as_str(),
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
    fn tree_hash_keeps_the_admitted_directory_when_its_name_is_replaced() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("object");
        let foreign = temporary.path().join("foreign");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&foreign).unwrap();
        std::fs::write(root.join("payload"), b"admitted").unwrap();
        std::fs::write(foreign.join("payload"), b"replacement").unwrap();
        let directory = gripsack_fs::open(&root).unwrap();
        let expected = canonical_tree_hash_in(&directory).unwrap();
        std::fs::rename(&root, temporary.path().join("retained")).unwrap();
        std::os::unix::fs::symlink(&foreign, &root).unwrap();
        assert_ne!(canonical_tree_hash(&root).unwrap(), expected);
        assert_eq!(canonical_tree_hash_in(&directory).unwrap(), expected);
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
            hash.as_str(),
            "cce3e9f819b476cc5abed85b83f2f1a01cac2abd4c2eb34f08b76d822739e595"
        );
    }
}
