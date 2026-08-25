//! `file:` — a tarball path or a plain directory of payload files.

use super::FetchError;
use super::archive;
#[cfg(test)]
use std::io::Write as _;
use std::path::Path;

pub(crate) fn fetch(path: &str, dest: &Path) -> Result<String, FetchError> {
    let path = Path::new(path);
    if path.is_dir() {
        archive::copy_tree(path, dest).map_err(FetchError::Io)?;
        Ok(gripsack_store::canonical_tree_hash(path)?)
    } else {
        let bytes = std::fs::read(path)?;
        let hash = archive::sha256(&bytes);
        let bare_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("bin");
        archive::extract(&bytes, dest, bare_name)?;
        Ok(hash)
    }
}

pub(crate) fn payload_hash(path: &str) -> Result<Option<String>, FetchError> {
    let path = Path::new(path);
    if path.is_dir() {
        Ok(Some(gripsack_store::canonical_tree_hash(path)?))
    } else {
        Ok(Some(archive::sha256(&std::fs::read(path)?)))
    }
}

#[cfg(test)]
pub(crate) fn make_tarball(path: &Path) {
    let file = std::fs::File::create(path).unwrap();
    let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(enc);
    let mut header = tar::Header::new_gnu();
    let content = b"#!/bin/sh\necho hello\n";
    header.set_size(content.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder
        .append_data(&mut header, "bin/hello", &content[..])
        .unwrap();
    builder
        .into_inner()
        .unwrap()
        .finish()
        .unwrap()
        .flush()
        .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_fetch_extracts_tarball() {
        let dir = tempfile::tempdir().unwrap();
        let tar = dir.path().join("hello.tar.gz");
        make_tarball(&tar);
        let dest = dir.path().join("out");
        fetch(&tar.to_string_lossy(), &dest).unwrap();
        assert!(dest.join("bin/hello").exists());
    }
}
