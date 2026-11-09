//! Checksum-verified release acquisition: the same steps as `install.sh`.
//! Resolve the version, download the tarball and its `.sha256` sidecar,
//! compare digests, and extract `grip` into the staging directory.

use gripsack_fetch::FetchContext;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const TARGET: &str = "x86_64-unknown-linux-musl";
const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;

/// Release downloads always come from GitHub; debug builds (used by tests)
/// may redirect them with `GRIPSACK_RELEASE_BASE_URL`.
fn base_url() -> String {
    #[cfg(debug_assertions)]
    if let Ok(base) = std::env::var("GRIPSACK_RELEASE_BASE_URL") {
        return base.trim_end_matches('/').to_owned();
    }
    "https://github.com/gripsack-dev/gripsack".to_owned()
}

fn ensure_supported_platform() -> Result<(), String> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Ok(())
    } else {
        Err("release downloads are published for Linux x86_64 only".into())
    }
}

/// The exact requested version, or the one GitHub's `releases/latest` redirects to.
pub(super) fn resolve(fetch: &FetchContext, requested: Option<&str>) -> Result<String, String> {
    if let Some(version) = requested {
        return Ok(version.to_owned());
    }
    let location = fetch
        .redirect_location(&format!("{}/releases/latest", base_url()))
        .map_err(|e| format!("cannot resolve the latest release: {e}"))?;
    let version = location
        .rsplit_once("/core-v")
        .map(|(_, version)| version)
        .filter(|version| super::parse_version(version).is_some())
        .ok_or_else(|| format!("unexpected latest-release redirect: {location}"))?;
    Ok(version.to_owned())
}

/// Download, verify and extract `grip` as `<staging>/grip`.
pub(super) fn stage(fetch: &FetchContext, version: &str, staging: &Path) -> Result<(), String> {
    ensure_supported_platform()?;
    let name = format!("gripsack-{version}-{TARGET}.tar.gz");
    let url = format!("{}/releases/download/core-v{version}/{name}", base_url());
    let tarball = fetch
        .download_public(&url)
        .map_err(|e| format!("cannot download {name}: {e}"))?;
    let sidecar = fetch
        .download_public(&format!("{url}.sha256"))
        .map_err(|e| format!("cannot download {name}.sha256: {e}"))?;
    let sidecar = std::fs::read_to_string(sidecar.file.path()).map_err(|e| e.to_string())?;
    let expected = expected_digest(&sidecar, &name)?;
    if tarball.hash.as_str() != expected {
        return Err(format!("checksum mismatch for {name}; nothing installed"));
    }
    extract_grip(tarball.file.path(), &staging.join("grip"))
}

/// `<hex>  <name>` as written by `sha256sum`.
fn expected_digest(sidecar: &str, name: &str) -> Result<String, String> {
    let mut words = sidecar.split_whitespace();
    let (Some(digest), Some(listed), None) = (words.next(), words.next(), words.next()) else {
        return Err("malformed checksum file".into());
    };
    if listed.trim_start_matches('*') != name
        || digest.len() != 64
        || !digest.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(format!("checksum file does not describe {name}"));
    }
    Ok(digest.to_ascii_lowercase())
}

fn extract_grip(tarball: &Path, output: &Path) -> Result<(), String> {
    let file = std::fs::File::open(tarball).map_err(|e| e.to_string())?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        if entry.header().entry_type().is_file() && path.file_name().is_some_and(|n| n == "grip") {
            let mut out = std::fs::File::create(output).map_err(|e| e.to_string())?;
            let mut limited = (&mut entry).take(MAX_BINARY_BYTES + 1);
            let copied = std::io::copy(&mut limited, &mut out).map_err(|e| e.to_string())?;
            if copied > MAX_BINARY_BYTES {
                return Err("grip binary in the release tarball is implausibly large".into());
            }
            out.flush().map_err(|e| e.to_string())?;
            std::fs::set_permissions(output, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err("the release tarball contains no grip binary".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_must_name_the_tarball_with_a_full_digest() {
        let digest = "a".repeat(64);
        assert_eq!(
            expected_digest(&format!("{digest}  x.tar.gz\n"), "x.tar.gz").unwrap(),
            digest
        );
        assert!(expected_digest(&format!("{digest}  y.tar.gz\n"), "x.tar.gz").is_err());
        assert!(expected_digest("abc  x.tar.gz\n", "x.tar.gz").is_err());
        assert!(expected_digest(&digest, "x.tar.gz").is_err());
    }
}
