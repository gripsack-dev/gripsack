//! Payload machinery: archive extraction (.tar.gz/.tar.xz/.zip/
    //! bare binaries) and the bottle pour (0002 §5).

use sha2::{Digest, Sha256};
use std::io;
use std::path::Path;

pub(crate) xtract_tarball(bytes: &[u8], dest: &Path) -> Result<(), FetchError> {
    const XZ_MAGIC: &[u8] = b"\xfd7zXZ\x00";
    const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
    if bytes.starts_with(XZ_MAGIC) {
        let mut archive = tar::Archive::new(xz2::read::XzDecoder::new(bytes));
        archive.unpack(dest)?;
        return Ok(());
    }
    if bytes.starts_with(ZIP_MAGIC) {
        zip::ZipArchive::new(io::Cursor::new(bytes))?.extract(dest)?;
        return Ok(());
    }
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut archive = tar::Archive::new(GzDecoder::new(bytes));
        archive.unpack(dest)?;
        return Ok(());
    }
    if bytes.starts_with(b"ustar".as_slice()) || looks_like_tar(bytes) {
        let mut archive = tar::Archive::new(bytes);
        archive.unpack(dest)?;
        return Ok(());
    }
    // bare payload: stage as one file, executable if it looks like a binary
    let path = dest.join("bin");
    std::fs::write(&path, bytes)?;
    #[cfg(unix)]
    if bytes.starts_with(b"\x7fELF") || bytes.starts_with(b"#!") {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn looks_like_tar(bytes: &[u8]) -> bool {
    bytes.len() > 262 && &bytes[257..262] == b"ustar"
}

/// The bottle pour: brew rewrites @@HOMEBREW_PREFIX@@ placeholders at
/// install time. For ELF binaries the interpreter is the blocker —
/// patch it to the system loader (fits the placeholder's length).
#[cfg(unix)]
fn pour(dest: &Path) -> io::Result<()> {
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
fn pour(_dest: &Path) -> io::Result<()> {
    Ok(())
}

pub(crate) opy_tree(from: &Path, to: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir_all(&target)?;
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

pub(crate) fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
