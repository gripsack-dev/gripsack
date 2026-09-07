//! Bottle acquisition: a locked URL is authoritative, including after the
//! registry's stable version moves. The transport digest precedes extraction.

use crate::{FetchContext, FetchError, FetchIdentity, FetchOutcome};
use std::path::Path;

pub(crate) fn fetch(
    context: &FetchContext,
    formula: &str,
    version: Option<&str>,
    sha256: Option<&str>,
    dest: &Path,
    locked: Option<&serde_json::Value>,
) -> Result<FetchOutcome, FetchError> {
    let field = |key| {
        locked
            .and_then(|pin| pin.get(key))
            .and_then(|value| value.as_str())
    };
    let resolved = match (field("url"), field("version")) {
        (Some(url), Some(version)) => crate::ResolvedRelease {
            url: url.into(),
            version: version.into(),
            api_url: None,
            sha256: field("sha256").map(str::to_owned),
        },
        _ => context
            .resolve_brew(formula)
            .map_err(|error| FetchError::Http {
                url: formula.into(),
                reason: error.to_string(),
            })?,
    };
    if let Some(expected) = version
        && expected != resolved.version
    {
        return Err(FetchError::Http {
            url: formula.into(),
            reason: format!(
                "bottle version {} does not match declared version {expected}",
                resolved.version
            ),
        });
    }
    let parsed = url::Url::parse(&resolved.url).map_err(|error| FetchError::Http {
        url: resolved.url.clone(),
        reason: error.to_string(),
    })?;
    let reader = if parsed.host_str() == Some("ghcr.io") {
        let scope = parsed
            .path()
            .strip_prefix("/v2/")
            .and_then(|path| path.split_once("/blobs/"))
            .map(|(repository, _)| repository)
            .ok_or_else(|| FetchError::Http {
                url: resolved.url.clone(),
                reason: "bottle URL has no registry repository".into(),
            })?;
        let token =
            crate::resolve::ghcr_token(context, scope).map_err(|error| FetchError::Http {
                url: resolved.url.clone(),
                reason: error.to_string(),
            })?;
        context
            .get(&resolved.url)
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(|error| FetchError::Http {
                url: resolved.url.clone(),
                reason: error.to_string(),
            })?
            .into_reader()
    } else {
        super::tarball::reader(context, &resolved.url, None)?
    };
    let mut download = crate::spool::download(reader, context.limits().download_bytes.get())?;
    let identity = FetchIdentity::Download(download.hash);
    crate::context::check_hash(
        &resolved.url,
        sha256.or(resolved.sha256.as_deref()),
        &identity,
    )?;
    super::archive::extract(
        download.file.as_file_mut(),
        dest,
        "bottle.tar",
        context.limits(),
    )?;
    super::archive::pour(dest)?;
    Ok(FetchOutcome {
        identity,
        url: Some(resolved.url),
        version: Some(resolved.version),
    })
}
