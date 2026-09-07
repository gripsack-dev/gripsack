//! Verified transport bytes are spooled before any archive is interpreted.

use crate::{DownloadHash, FetchContext, FetchError, FetchIdentity, FetchOutcome};
use std::io::{self, Read};
use std::path::Path;

pub(crate) fn fetch(
    context: &FetchContext,
    url: &str,
    expected: Option<&str>,
    api_url: Option<&str>,
    dest: &Path,
) -> Result<FetchOutcome, FetchError> {
    let mut download = crate::spool::download(
        reader(context, url, api_url)?,
        context.limits().download_bytes.get(),
    )?;
    if let Some(expected) = expected
        && expected != download.hash.as_str()
    {
        return Err(FetchError::HashMismatch {
            url: url.into(),
            expected: expected.into(),
            actual: download.hash.into(),
        });
    }
    let bare_name = url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("bin");
    super::archive::extract(
        download.file.as_file_mut(),
        dest,
        bare_name,
        context.limits(),
    )?;
    Ok(FetchOutcome {
        identity: FetchIdentity::Download(download.hash),
        url: None,
        version: None,
    })
}

pub(crate) fn payload_hash(
    context: &FetchContext,
    url: &str,
    api_url: Option<&str>,
) -> Result<DownloadHash, FetchError> {
    crate::spool::copy_hashed(
        reader(context, url, api_url)?,
        io::sink(),
        context.limits().download_bytes.get(),
    )
}

pub(crate) fn reader(
    context: &FetchContext,
    url: &str,
    api_url: Option<&str>,
) -> Result<Box<dyn Read>, FetchError> {
    if let Some(path) = url.strip_prefix("file://") {
        return regular_file(Path::new(path)).map(|file| Box::new(file) as Box<dyn Read>);
    }
    let via_api = api_url.filter(|api| context.auth_header(api).is_some());
    let request_url = via_api.unwrap_or(url);
    let mut request = context.get(request_url);
    if let Some(header) = context.auth_header(request_url) {
        request = request.set("Authorization", header);
    }
    if via_api.is_some() {
        request = request.set("Accept", "application/octet-stream");
    }
    let response = request.call().map_err(|error| FetchError::Http {
        url: request_url.into(),
        reason: error.to_string(),
    })?;
    if response
        .header("content-type")
        .unwrap_or_default()
        .contains("text/html")
    {
        return Err(FetchError::Http { url: request_url.into(), reason: "server returned an HTML/login page instead of an asset; check host-bound authentication".into() });
    }
    Ok(response.into_reader())
}

pub(crate) fn regular_file(path: &Path) -> Result<std::fs::File, FetchError> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular payload file", path.display()),
        )
        .into());
    }
    Ok(file)
}

pub(crate) fn text(
    context: &FetchContext,
    url: &str,
    api_url: Option<&str>,
) -> Result<String, FetchError> {
    let mut result = String::new();
    crate::spool::Limited::new(
        reader(context, url, api_url)?,
        8 * 1024 * 1024,
        "registry metadata",
    )
    .read_to_string(&mut result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_ir::FetchSpec;

    #[test]
    fn transport_mismatch_never_extracts_payload() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = temporary.path().join("hello.tar.gz");
        super::super::file::make_tarball(&archive);
        let dest = temporary.path().join("out");
        let context = FetchContext::default();
        let spec = FetchSpec::Tarball {
            url: format!("file://{}", archive.display()),
            sha256: Some("0".repeat(64)),
            api_url: None,
        };
        assert!(matches!(
            context.fetch(&spec, &dest, None),
            Err(FetchError::HashMismatch { .. })
        ));
        assert!(!dest.join("bin/hello").exists());
    }
}
