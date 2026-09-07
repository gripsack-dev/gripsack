//! `file:` — a tarball path or a plain directory of payload files.

use super::FetchError;
use super::archive;
#[cfg(test)]
use std::io::Write as _;
use std::path::Path;

pub(crate) fn fetch(
    context: &crate::FetchContext,
    path: &str,
    dest: &Path,
) -> Result<crate::FetchOutcome, FetchError> {
    let path = Path::new(path);
    let identity = if std::fs::metadata(path)?.is_dir() {
        archive::copy_tree_filtered(path, dest, &[], context.limits())?;
        archive::validate_tree(dest, context.limits())?;
        crate::FetchIdentity::Tree(gripsack_store::canonical_tree_hash(dest)?)
    } else {
        let mut download = crate::spool::download(
            super::tarball::regular_file(path)?,
            context.limits().download_bytes.get(),
        )?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("bin");
        archive::extract(download.file.as_file_mut(), dest, name, context.limits())?;
        crate::FetchIdentity::Download(download.hash)
    };
    Ok(crate::FetchOutcome {
        identity,
        url: None,
        version: None,
    })
}

pub(crate) fn payload_hash(
    context: &crate::FetchContext,
    path: &str,
) -> Result<crate::FetchIdentity, FetchError> {
    let path = Path::new(path);
    if std::fs::metadata(path)?.is_dir() {
        archive::validate_tree(path, context.limits())?;
        Ok(crate::FetchIdentity::Tree(
            gripsack_store::canonical_tree_hash(path)?,
        ))
    } else {
        crate::spool::copy_hashed(
            super::tarball::regular_file(path)?,
            std::io::sink(),
            context.limits().download_bytes.get(),
        )
        .map(crate::FetchIdentity::Download)
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
        fetch(
            &crate::FetchContext::default(),
            &tar.to_string_lossy(),
            &dest,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(dest.join("bin/hello")).unwrap(),
            b"#!/bin/sh\necho hello\n"
        );
    }
}
