//! Payload machinery (0009 critique): archive extraction
//! (.tar.gz/.tar/.tar.xz/.zip), bare binaries staged as a single
//! executable file, and the bottle pour.

use super::FetchError;
use sha2::{Digest, Sha256};
use std::io;
use std::path::Path;

/// `bare_name` names the staged file when the payload isn't an
/// archive — the asset's filename from the URL (falling back to
/// "bin"), never a hardcoded name that collides with tarball bin/
/// directories (review finding F5).
pub(crate) fn extract(bytes: &[u8], dest: &Path, bare_name: &str) -> Result<(), FetchError> {
    const XZ_MAGIC: &[u8] = b"\xfd7zXZ\x00";
    const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
    if bytes.starts_with(XZ_MAGIC) {
        let mut archive = tar::Archive::new(xz2::read::XzDecoder::new(bytes));
        if archive.unpack(dest).is_ok() {
            return Ok(());
        }
        // a single .xz file, not a tar.xz — decompress and stage bare
        let raw = decompress(xz2::read::XzDecoder::new(bytes))?;
        return stage_bare(&raw, dest, strip_suffix(bare_name, ".xz"));
    }
    if bytes.starts_with(ZIP_MAGIC) {
        zip::ZipArchive::new(io::Cursor::new(bytes))?.extract(dest)?;
        return Ok(());
    }
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
        if archive.unpack(dest).is_ok() {
            return Ok(());
        }
        // a single .gz file, not a tar.gz (e.g. tree-sitter's bare
        // binary) — decompress and stage as one file (finding: the
        // archive walk failed with 'failed to iterate over archive')
        let raw = decompress(flate2::read::GzDecoder::new(bytes))?;
        return stage_bare(&raw, dest, strip_suffix(bare_name, ".gz"));
    }
    if looks_like_tar(bytes) {
        let mut archive = tar::Archive::new(bytes);
        archive.unpack(dest)?;
        return Ok(());
    }
    // bare payload: stage as one file named after the asset,
    // executable if it looks like a binary
    stage_bare(bytes, dest, bare_name)
}

fn stage_bare(bytes: &[u8], dest: &Path, bare_name: &str) -> Result<(), FetchError> {
    let path = dest.join(bare_name);
    std::fs::write(&path, bytes)?;
    #[cfg(unix)]
    if bytes.starts_with(b"\x7fELF") || bytes.starts_with(b"#!") {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn decompress<R: io::Read>(mut r: R) -> Result<Vec<u8>, FetchError> {
    let mut out = Vec::new();
    io::Read::read_to_end(&mut r, &mut out)?;
    Ok(out)
}

fn strip_suffix<'a>(name: &'a str, suffix: &str) -> &'a str {
    name.strip_suffix(suffix).unwrap_or(name)
}

fn looks_like_tar(bytes: &[u8]) -> bool {
    bytes.len() > 262 && &bytes[257..262] == b"ustar"
}

/// The bottle pour: brew rewrites @@HOMEBREW_PREFIX@@ placeholders at
/// install time. For ELF binaries the interpreter is the blocker —
/// patch it to the system loader (fits the placeholder's length).
#[cfg(unix)]
pub(crate) fn pour(dest: &Path) -> io::Result<()> {
    const PLACEHOLDER: &[u8] = b"@@HOMEBREW_PREFIX@@/lib/ld.so";
    let loader: &[u8] = if cfg!(target_arch = "aarch64") {
        b"/lib/ld-linux-aarch64.so.1"
    } else {
        b"/lib64/ld-linux-x86-64.so.2"
    };
    pour_dir(dest, PLACEHOLDER, loader)
}

#[cfg(unix)]
fn pour_dir(dir: &Path, placeholder: &[u8], loader: &[u8]) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() && !path.is_symlink() {
            pour_dir(&path, placeholder, loader)?;
        } else if path.is_file() {
            let mut bytes = std::fs::read(&path)?;
            let mut cursor = 0;
            let mut touched = false;
            while let Some(pos) = bytes[cursor..]
                .windows(placeholder.len())
                .position(|w| w == placeholder)
            {
                let start = cursor + pos;
                bytes[start..start + loader.len()].copy_from_slice(loader);
                for b in &mut bytes[start + loader.len()..start + placeholder.len()] {
                    *b = 0;
                }
                cursor = start + placeholder.len();
                touched = true;
            }
            if touched {
                // bottles ship read-only files
                let mut perms = std::fs::metadata(&path)?.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                perms.set_readonly(false);
                std::fs::set_permissions(&path, perms)?;
                std::fs::write(&path, &bytes)?;
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn pour(_dest: &Path) -> io::Result<()> {
    Ok(())
}

pub(crate) fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    copy_tree_filtered(from, to, &[])
}

/// Copy a tree, skipping top-level entries named in `skip`.
pub(crate) fn copy_tree_filtered(from: &Path, to: &Path, skip: &[&str]) -> io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        if skip.iter().any(|s| entry.file_name() == *s) {
            continue;
        }
        let target = to.join(entry.file_name());
        let path = entry.path();
        let file_type = entry.file_type()?;
        // io errors carry the path or they're archaeology bait (the
        // tmux/conda terminfo lesson: `io: the source path is neither
        // a regular file…` with no path named)
        let locate = |e: io::Error| io::Error::new(e.kind(), format!("{}: {e}", path.display()));
        if file_type.is_symlink() {
            // relink, never follow: a symlink TO A DIRECTORY would be
            // miscopied (fs::copy rejects non-file sources), and the
            // canonical identity covers the target anyway
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(&path).map_err(locate)?, &target)
                .map_err(locate)?;
            #[cfg(not(unix))]
            std::fs::copy(&path, &target).map_err(locate)?;
        } else if file_type.is_dir() {
            std::fs::create_dir_all(&target).map_err(locate)?;
            copy_tree_filtered(&path, &target, skip)?;
        } else {
            std::fs::copy(&path, &target).map_err(locate)?;
        }
    }
    Ok(())
}

pub(crate) fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn bare_binary_staged_as_executable_file() {
        let dir = tempfile::tempdir().unwrap();
        extract(b"\x7fELF-fake-binary", dir.path(), "tool").unwrap();
        let staged = dir.path().join("tool");
        assert_eq!(std::fs::read(&staged).unwrap(), b"\x7fELF-fake-binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                staged.metadata().unwrap().permissions().mode() & 0o111,
                0o111
            );
        }
    }

    #[test]
    fn xz_extracts() {
        let dir = tempfile::tempdir().unwrap();
        let xz_path = dir.path().join("p.tar.xz");
        {
            let file = std::fs::File::create(&xz_path).unwrap();
            let enc = xz2::write::XzEncoder::new(file, 6);
            let mut builder = tar::Builder::new(enc);
            let mut header = tar::Header::new_gnu();
            header.set_size(1);
            header.set_cksum();
            builder
                .append_data(&mut header, "x.txt", &b"x"[..])
                .unwrap();
            builder
                .into_inner()
                .unwrap()
                .finish()
                .unwrap()
                .flush()
                .unwrap();
        }
        let out = dir.path().join("out");
        extract(&std::fs::read(&xz_path).unwrap(), &out, "bin").unwrap();
        assert!(out.join("x.txt").exists());
    }
}

#[cfg(test)]
mod single_file_tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn single_gz_file_decompresses_and_stages_bare() {
        let dir = tempfile::tempdir().unwrap();
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(b"\x7fELF fake binary").unwrap();
        let gz = enc.finish().unwrap();
        extract(&gz, dir.path(), "tree-sitter-linux-x64.gz").unwrap();
        let staged = dir.path().join("tree-sitter-linux-x64");
        assert_eq!(std::fs::read(&staged).unwrap(), b"\x7fELF fake binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                staged.metadata().unwrap().permissions().mode() & 0o111,
                0o111
            );
        }
    }
}
