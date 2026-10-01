//! Lazy provisioning of the pinned BuildKit helper, reached only by a real
//! protected solve. Native/prebuilt/check/preview/rollback paths never enter it.
//!
//! Trust shape (host.rs): the per-platform sha256 is compiled into the
//! binary and the released asset is the raw static executable, so the
//! bytes on disk are verified against the pin on every use. Corrupt owned bytes
//! are replaced; foreign aliases are refused. The lock and every cache access
//! are relative to pinned directory capabilities. Streamed publication uses a
//! private create-new sibling, SHA verification, mode, fsync and atomic rename.
//! Unknown remnants are left untouched, never treated as ours by filename alone.
//!
//! `GRIPSACK_BRIDGE_MIRROR` declares an operator mirror of the exact
//! pinned artifact bytes (a `file://` or `https://` base under which
//! the same asset name lives). It swaps ONLY the download origin — the
//! compile-time pin still authenticates the bytes, so a mirror serving
//! anything else fails closed. Runtime qualification uses it to
//! exercise this exact production path against unpublishable bytes.

use super::tarball;
mod checked;
mod state;
use state::ToolSlot;
use crate::host;
use crate::{FetchContext, FetchError};
use std::io::Seek;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The provisioned helper: its path and the compile-time pin its bytes
/// were verified against. Callers bind process selection to the same
/// digest, closing the provision → exec window.
#[derive(Debug)]
pub struct ProvisionedBridge {
    pub path: PathBuf,
    pub pin: String,
}

/// The pinned helper for this core release, provisioned on first use.
/// Fail-closed throughout: no pin for this platform (the helper release
/// is not published yet), corrupted cache bytes, a symlinked cache
/// entry, and download/hash failures all error with the remedy named.
pub fn ensure(home: &Path, context: &FetchContext, deadline: Instant) -> Result<ProvisionedBridge, FetchError> {
    let (url, pin) = host::resolve(&host::BRIDGE_RELEASE).map_err(|error| match error {
        FetchError::Source { resource, reason } => FetchError::Source {
            resource,
            reason: format!(
                "{reason}; until the helper's per-platform pins for this release are published, \
                 pass --bridge to select a deliberate operator override"
            ),
        },
        other => other,
    })?;
    let url = match std::env::var_os("GRIPSACK_BRIDGE_MIRROR") {
        Some(mirror) => {
            let mirror = mirror.into_string().map_err(|_| FetchError::Source {
                resource: host::BRIDGE_RELEASE.url_template.into(),
                reason: "GRIPSACK_BRIDGE_MIRROR must be valid UTF-8".into(),
            })?;
            let asset = url.rsplit('/').next().expect("release asset name");
            // Origin moves, trust does not: the compiled-in pin still
            // authenticates the bytes. Logged, never silent.
            tracing::info!(asset, "bridge download origin redirected by GRIPSACK_BRIDGE_MIRROR");
            format!("{}/{asset}", mirror.trim_end_matches('/'))
        }
        None => url,
    };
    ensure_release(
        home,
        context.provisioning(),
        host::BRIDGE_RELEASE.version,
        &url,
        pin,
        deadline,
    )
}

/// What the cache check found at the versioned executable path.
enum Warm {
    /// Bytes on disk match the compile-time pin.
    Ready,
    /// Nothing there (first run, or a crash left only an empty dir).
    Missing,
    /// Regular file whose bytes do NOT match the pin (its actual hash).
    Corrupt(String),
}

fn ensure_release(
    home: &Path,
    context: &FetchContext,
    version: &str,
    url: &str,
    pin: &str,
    deadline: Instant,
) -> Result<ProvisionedBridge, FetchError> {
    let slot = ToolSlot::open(home, version, deadline)?;
    let executable = &slot.executable;
    let evicted = match warm(&slot, pin, context)? {
        Warm::Ready => return Ok(provisioned(executable.clone(), pin)),
        Warm::Missing => None,
        Warm::Corrupt(actual) => {
            gripsack_fs::remove_file(&slot.directory, Path::new("bridge"))?;
            Some(actual)
        }
    };
    provision(&slot, url, pin, context, deadline).map_err(|error| {
        match evicted {
            Some(actual) => FetchError::Source {
                resource: crate::http::safe_location(url).into_owned(),
                reason: format!(
                    "cached bridge at {} failed its pinned sha256 check (expected {pin}, got {actual}) \
                     and re-provisioning failed: {error}; check the network and retry, or remove the \
                     cache entry and pass --bridge to select a deliberate operator override",
                    executable.display()
                ),
            },
            None => FetchError::Source {
                resource: crate::http::safe_location(url).into_owned(),
                reason: format!(
                    "the pinned BuildKit bridge {} is not provisioned at {} and could not be acquired: \
                     {error}; check the network (or GRIPSACK_BRIDGE_MIRROR) and retry, or pass \
                     --bridge to select a deliberate operator override",
                    version,
                    executable.display()
                ),
            },
        }
    })
}

/// Publish only through the existing capability-relative streamed transaction.
/// Unrecognized sibling files are not reclaimed based on their spelling.
fn provision(
    slot: &ToolSlot,
    url: &str,
    pin: &str,
    context: &FetchContext,
    deadline: Instant,
) -> Result<ProvisionedBridge, FetchError> {
    if Instant::now() >= deadline {
        return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "bridge provisioning deadline expired").into());
    }
    let mut download = tarball::download(context, url, None)?;
    if download.hash.as_str() != pin {
        return Err(FetchError::HashMismatch {
            url: url.into(), expected: pin.into(), actual: download.hash.as_str().into(),
        });
    }
    download.file.as_file_mut().rewind()?;
    let expected = gripsack_process::Sha256Digest::parse(pin)?;
    let mut checked = checked::CheckedRead::new(
        download.file.as_file_mut(), expected, context.limits().download_bytes.get(), deadline,
    );
    gripsack_fs::atomic_copy_with_mode(&slot.directory, Path::new("bridge"), &mut checked, 0o755)?;
    Ok(provisioned(slot.executable.clone(), pin))
}

/// The cache check every use runs: owned regular bytes whose measured
/// sha256 equals the pin. Offline — no network is touched here.
fn warm(slot: &ToolSlot, pin: &str, context: &FetchContext) -> Result<Warm, FetchError> {
    use gripsack_fs::cap_std::fs::{MetadataExt, PermissionsExt};
    let file = match gripsack_fs::open_file_nofollow(&slot.directory, Path::new("bridge")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Warm::Missing),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no preconditions.
    if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(FetchError::Source {
            resource: slot.executable.display().to_string(),
            reason: "bridge cache must be an owned regular file, not writable by another user".into(),
        });
    }
    let actual = measured_hash(file.into_std(), context)?;
    Ok(if actual == pin { Warm::Ready } else { Warm::Corrupt(actual) })
}

/// Streaming sha256 of the cached executable, capped by the same
/// download bound that would govern its replacement.
fn measured_hash(file: std::fs::File, context: &FetchContext) -> Result<String, FetchError> {
    use sha2::Digest;
    use std::io::Read;
    let mut source = crate::spool::Limited::new(
        file,
        context.limits().download_bytes.get(),
        "buildkit bridge cache",
    );
    let mut hasher = sha2::Sha256::new();
    let mut chunk = [0; 64 * 1024];
    loop {
        let read = source.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(hex, "{byte:02x}").expect("hex into String");
    }
    Ok(hex)
}


fn provisioned(path: PathBuf, pin: &str) -> ProvisionedBridge {
    ProvisionedBridge {
        path,
        pin: pin.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: &str = "9.9.9-test";

    /// `file://`-served fixture bytes plus their measured pin: the same
    /// shape a declared operator mirror serves in qualification.
    fn fixture(bytes: &[u8]) -> (String, String, tempfile::TempDir) {
        let serving = tempfile::tempdir().unwrap();
        let asset = serving.path().join("bridge-fixture");
        std::fs::write(&asset, bytes).unwrap();
        let pin = gripsack_store::hash::hex_sha256(bytes);
        (format!("file://{}", asset.display()), pin, serving)
    }

    fn ensure_with(home: &Path, url: &str, pin: &str) -> Result<ProvisionedBridge, FetchError> {
        ensure_release(home, &FetchContext::default(), VERSION, url, pin, Instant::now() + std::time::Duration::from_secs(30))
    }

    #[test]
    fn cold_provisions_verified_bytes() {
        let home = tempfile::tempdir().unwrap();
        let (url, pin, _serving) = fixture(b"pinned bridge bytes");
        let provisioned = ensure_with(home.path(), &url, &pin).unwrap();
        assert_eq!(provisioned.pin, pin);
        assert_eq!(
            std::fs::read(&provisioned.path).unwrap(),
            b"pinned bridge bytes"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&provisioned.path).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
    }

    #[test]
    fn warm_reuse_is_offline() {
        let home = tempfile::tempdir().unwrap();
        let (url, pin, _serving) = fixture(b"pinned bridge bytes");
        let first = ensure_with(home.path(), &url, &pin).unwrap();
        // Same version + pins, dead origin: reuse must not fetch.
        let second = ensure_with(home.path(), "file:///definitely/gone", &pin).unwrap();
        assert_eq!(first.path, second.path);
    }

    #[test]
    fn corrupted_cache_reprovisions_when_online() {
        let home = tempfile::tempdir().unwrap();
        let (url, pin, _serving) = fixture(b"pinned bridge bytes");
        let first = ensure_with(home.path(), &url, &pin).unwrap();
        std::fs::write(&first.path, b"attacker-controlled bytes").unwrap();
        let second = ensure_with(home.path(), &url, &pin).unwrap();
        assert_eq!(std::fs::read(&second.path).unwrap(), b"pinned bridge bytes");
    }

    #[test]
    fn corrupted_cache_offline_fails_closed() {
        let home = tempfile::tempdir().unwrap();
        let (url, pin, _serving) = fixture(b"pinned bridge bytes");
        let first = ensure_with(home.path(), &url, &pin).unwrap();
        std::fs::write(&first.path, b"attacker-controlled bytes").unwrap();
        assert!(ensure_with(home.path(), "file:///definitely/gone", &pin).is_err());
        // The corrupt bytes are gone; nothing unverified remains.
        assert!(!first.path.exists());
    }


    #[test]
    fn unproven_staging_is_preserved_without_blocking_publication() {
        let home = tempfile::tempdir().unwrap();
        let (url, pin, _serving) = fixture(b"pinned bridge bytes");
        let tools = home.path().join("tools");
        // A prefix is not ownership evidence: this may be crash debris or an
        // unrelated operator directory. A fresh publication must not erase it.
        let orphan = tools.join(".bridge-deadbeef");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(orphan.join("bridge"), b"torn").unwrap();
        std::fs::create_dir_all(tools.join(format!("buildkit-bridge-{VERSION}"))).unwrap();
        let provisioned = ensure_with(home.path(), &url, &pin).unwrap();
        assert_eq!(std::fs::read(&provisioned.path).unwrap(), b"pinned bridge bytes");
        assert_eq!(std::fs::read(orphan.join("bridge")).unwrap(), b"torn");
    }

    #[test]
    fn symlinked_cache_is_refused_not_followed() {
        let home = tempfile::tempdir().unwrap();
        let (url, pin, _serving) = fixture(b"pinned bridge bytes");
        let executable = home
            .path()
            .join("tools")
            .join(format!("buildkit-bridge-{VERSION}"))
            .join("bridge");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        let target = home.path().join("elsewhere");
        std::fs::write(&target, b"pinned bridge bytes").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &executable).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&target, &executable).unwrap();
        assert!(ensure_with(home.path(), &url, &pin).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"pinned bridge bytes");
        // The refusal must not delete or follow the redirect.
        assert!(std::fs::symlink_metadata(&executable)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn parent_namespace_aliases_cannot_redirect_provisioning() {
        for relative in ["tools".to_owned(), "locks".to_owned(), format!("tools/buildkit-bridge-{VERSION}")] {
            let home = tempfile::tempdir().unwrap();
            let foreign = tempfile::tempdir().unwrap();
            let (url, pin, _serving) = fixture(b"pinned bridge bytes");
            std::fs::write(foreign.path().join("sentinel"), b"untouched").unwrap();
            let alias = home.path().join(relative);
            std::fs::create_dir_all(alias.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(foreign.path(), &alias).unwrap();
            assert!(ensure_with(home.path(), &url, &pin).is_err());
            assert_eq!(std::fs::read(foreign.path().join("sentinel")).unwrap(), b"untouched");
            assert_eq!(std::fs::read_dir(foreign.path()).unwrap().count(), 1);
            assert!(alias.symlink_metadata().unwrap().is_symlink());
        }
    }

    #[test]
    fn download_mismatch_never_lands_bytes() {
        let home = tempfile::tempdir().unwrap();
        let (url, _pin, _serving) = fixture(b"served bytes");
        let wrong = "0".repeat(64);
        let error = ensure_with(home.path(), &url, &wrong).unwrap_err();
        assert!(matches!(error, FetchError::Source { .. }), "{error}");
        let executable = home
            .path()
            .join("tools")
            .join(format!("buildkit-bridge-{VERSION}"))
            .join("bridge");
        assert!(!executable.exists());
    }
}
