//! `tarball:` — a URL (file:// or https) to an archive or bare binary.

use super::FetchError;
use super::archive;
use std::io::Read as _;
use std::path::Path;

pub(crate) fn fetch(url: &str, sha256: Option<&str>, dest: &Path) -> Result<String, FetchError> {
    let bytes = read_url(url)?;
    let actual = archive::sha256(&bytes);
    if let Some(expected) = sha256
        && actual != *expected
    {
        return Err(FetchError::HashMismatch {
            url: url.to_string(),
            expected: expected.to_string(),
            actual,
        });
    }
    archive::extract(&bytes, dest)?;
    Ok(actual)
}

pub(crate) fn payload_hash(url: &str) -> Result<Option<String>, FetchError> {
    Ok(Some(archive::sha256(&read_url(url)?)))
}

pub(crate) fn read_url(url: &str) -> Result<Vec<u8>, FetchError> {
    if let Some(path) = url.strip_prefix("file://") {
        return Ok(std::fs::read(path)?);
    }
    let response = crate::http::agent()
        .get(url)
        .call()
        .map_err(|e| FetchError::Http {
            url: url.to_string(),
            reason: e.to_string(),
        })?;
    let reader = response.into_reader();
    let mut limited = io::Read::take(reader, 512 * 1024 * 1024);
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .map_err(|e| FetchError::Http {
            url: url.to_string(),
            reason: e.to_string(),
        })?;
    Ok(bytes)
}

use std::io;

#[cfg(test)]
mod tests {
    use super::super::FetchSpec;
    use super::super::fetch;
    use super::super::file::make_tarball;
    use super::archive::sha256;
    use super::*;

    #[test]
    fn tarball_file_url_verifies_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let tar = dir.path().join("hello.tar.gz");
        make_tarball(&tar);
        let url = format!("file://{}", tar.display());
        let good = sha256(&std::fs::read(&tar).unwrap());
        let dest = dir.path().join("out");
        fetch(
            &FetchSpec::Tarball {
                url: url.clone(),
                sha256: Some(good),
            },
            &dest,
        )
        .unwrap();
        assert!(dest.join("bin/hello").exists());

        let err = fetch(
            &FetchSpec::Tarball {
                url,
                sha256: Some("0".repeat(64)),
            },
            &dir.path().join("out2"),
        )
        .unwrap_err();
        assert!(matches!(err, FetchError::HashMismatch { .. }));
    }
}
