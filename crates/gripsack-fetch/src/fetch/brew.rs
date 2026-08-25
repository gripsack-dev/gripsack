//! `brew:` — Homebrew bottles. Resolution is pure formula JSON (the
//! bottle sha256 pins without a download); the blob needs a ghcr
//! anonymous token; the pour rewrites the @@HOMEBREW_PREFIX@@ loader.

use super::FetchError;
use super::archive;
use std::io;
use std::path::Path;

pub(crate) fn fetch(
    formula: &str,
    sha256: Option<&str>,
    dest: &Path,
) -> Result<String, FetchError> {
    let resolved = crate::resolve::resolve_brew(formula).map_err(|e| FetchError::Http {
        url: formula.to_string(),
        reason: e.to_string(),
    })?;
    let token = crate::resolve::ghcr_token(&format!("homebrew/core/{formula}")).map_err(|e| {
        FetchError::Http {
            url: formula.to_string(),
            reason: e.to_string(),
        }
    })?;
    let bytes = crate::http::agent()
        .get(&resolved.url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| FetchError::Http {
            url: resolved.url.clone(),
            reason: e.to_string(),
        })?
        .into_reader();
    let mut bytes_vec = Vec::new();
    io::Read::read_to_end(
        &mut io::Read::take(bytes, 512 * 1024 * 1024),
        &mut bytes_vec,
    )?;
    let actual = archive::sha256(&bytes_vec);
    // the lock pin wins over the formula's own hash (0002 §3)
    let expected = sha256.or(resolved.sha256.as_deref());
    if let Some(expected) = expected
        && actual != *expected
    {
        return Err(FetchError::HashMismatch {
            url: resolved.url,
            expected: expected.to_string(),
            actual,
        });
    }
    archive::extract(&bytes_vec, dest)?;
    archive::pour(dest)?;
    Ok(actual)
}

pub(crate) fn payload_hash(formula: &str) -> Result<Option<String>, FetchError> {
    Ok(crate::resolve::resolve_brew(formula)
        .map_err(|e| FetchError::Http {
            url: formula.to_string(),
            reason: e.to_string(),
        })?
        .sha256)
}
