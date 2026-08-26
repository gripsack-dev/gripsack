//! `tarball:` — a URL (file:// or https) to an archive or bare binary.

use super::FetchError;
use super::archive;
use std::io::Read as _;
use std::path::Path;

pub(crate) fn fetch(
    url: &str,
    sha256: Option<&str>,
    api_url: Option<&str>,
    dest: &Path,
) -> Result<String, FetchError> {
    let bytes = read_url_authed(url, api_url)?;
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
    let bare_name = url
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("bin");
    archive::extract(&bytes, dest, bare_name)?;
    Ok(actual)
}

pub(crate) fn payload_hash(url: &str, api_url: Option<&str>) -> Result<Option<String>, FetchError> {
    Ok(Some(archive::sha256(&read_url_authed(url, api_url)?)))
}

/// Download with host-scoped auth. When the spec carries an API asset
/// endpoint (github releases) AND a token is bound to that host, the
/// download goes through the API with `Accept: application/octet-stream`
/// — the browser URL needs a browser session on private/GHE releases
/// and returns a SAML login page instead of bytes (enterprise finding
/// #1). A `text/html` response is a login page, never an asset — fail
/// with the cause, not a misleading hash mismatch.
pub(crate) fn read_url_authed(url: &str, api_url: Option<&str>) -> Result<Vec<u8>, FetchError> {
    if url.starts_with("file://") {
        return read_url(url);
    }
    let via_api = api_url
        .and_then(|api| crate::http::auth_header(api).map(|header| (api.to_string(), header)));
    let (get_url, accept_octet) = match &via_api {
        Some((api, _)) => (api.clone(), true),
        None => (url.to_string(), false),
    };
    let mut request = crate::http::get(&get_url);
    if let Some((_, header)) = &via_api {
        request = request.set("Authorization", header);
    } else if let Some(header) = crate::http::auth_header(&get_url) {
        request = request.set("Authorization", &header);
    }
    if accept_octet {
        request = request.set("Accept", "application/octet-stream");
    }
    let response = request.call().map_err(|e| FetchError::Http {
        url: get_url.clone(),
        reason: e.to_string(),
    })?;
    if response
        .header("content-type")
        .unwrap_or_default()
        .contains("text/html")
    {
        return Err(FetchError::Http {
            url: get_url,
            reason: "the server returned an HTML page, not an asset — this looks \
                     like a login/SSO redirect (a private release fetched without \
                     a bound token?)"
                .into(),
        });
    }
    let reader = response.into_reader();
    let mut limited = io::Read::take(reader, 512 * 1024 * 1024);
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .map_err(|e| FetchError::Http {
            url: get_url,
            reason: e.to_string(),
        })?;
    Ok(bytes)
}

pub(crate) fn read_url(url: &str) -> Result<Vec<u8>, FetchError> {
    if let Some(path) = url.strip_prefix("file://") {
        return Ok(std::fs::read(path)?);
    }
    let response = crate::http::get(url).call().map_err(|e| FetchError::Http {
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
                api_url: None,
            },
            &dest,
        )
        .unwrap();
        assert!(dest.join("bin/hello").exists());

        let err = fetch(
            &FetchSpec::Tarball {
                url,
                sha256: Some("0".repeat(64)),
                api_url: None,
            },
            &dir.path().join("out2"),
        )
        .unwrap_err();
        assert!(matches!(err, FetchError::HashMismatch { .. }));
    }
}
