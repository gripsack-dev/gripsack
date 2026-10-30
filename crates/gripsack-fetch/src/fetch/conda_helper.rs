//! Lazy provisioning of the pinned Conda helper.
//! Native/prebuilt/check/preview/rollback paths never enter it.
//!
//! Trust shape (host.rs): the per-platform sha256 is compiled into the
//! binary and the released asset is the raw static executable, so the
//! bytes on disk are verified against the pin on every use. Corrupt owned bytes
//! are replaced; foreign aliases are refused. The lock and every cache access
//! are relative to pinned directory capabilities. Streamed publication uses a
//! private create-new sibling, SHA verification, mode, fsync and atomic rename.
//! Unknown remnants are left untouched, never treated as ours by filename alone.
//!
//! `GRIPSACK_CONDA_HELPER_MIRROR` declares an operator mirror of the exact
//! pinned artifact bytes (a `file://` or `https://` base under which
//! the same asset name lives). It swaps ONLY the download origin — the
//! compile-time pin still authenticates the bytes, so a mirror serving
//! anything else fails closed. Runtime qualification uses it to
//! exercise this exact production path against unpublishable bytes.

mod checked;
mod state;
use crate::host;
use crate::{FetchContext, FetchError};
use state::ToolSlot;
use std::io::Seek;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Provision the Conda helper using its compiled-in pinned-byte authority.
pub fn ensure(
    home: &Path,
    context: &FetchContext,
    deadline: Instant,
) -> Result<ProvisionedHelper, FetchError> {
    let (url, pin) = host::resolve(&host::CONDA_RELEASE)?;
    let url = match std::env::var_os("GRIPSACK_CONDA_HELPER_MIRROR") {
        Some(mirror) => {
            let mirror = mirror.into_string().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "GRIPSACK_CONDA_HELPER_MIRROR must be UTF-8",
                )
            })?;
            let origin = if Path::new(&mirror).is_absolute() {
                format!("file://{mirror}")
            } else {
                mirror
            };
            format!(
                "{}/{}",
                origin.trim_end_matches('/'),
                url.rsplit('/').next().expect("asset name")
            )
        }
        None => url,
    };
    ensure_release(
        home,
        context.provisioning(),
        host::CONDA_RELEASE.version,
        &url,
        pin,
        deadline,
    )
}

/// The provisioned helper: its path and the compile-time pin its bytes
/// were verified against. Callers bind process selection to the same
/// digest, closing the provision → exec window.
#[derive(Debug)]
pub struct ProvisionedHelper {
    pub path: PathBuf,
    pub pin: String,
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
) -> Result<ProvisionedHelper, FetchError> {
    let slot = ToolSlot::open(home, version, deadline)?;
    let executable = &slot.executable;
    let evicted = match warm(&slot, pin, context, deadline)? {
        Warm::Ready => return Ok(provisioned(executable.clone(), pin)),
        Warm::Missing => None,
        Warm::Corrupt(actual) => {
            gripsack_fs::remove_file(&slot.directory, Path::new(slot.name))?;
            Some(actual)
        }
    };
    provision(&slot, url, pin, context, deadline).map_err(|error| {
        match evicted {
            Some(actual) => FetchError::Source {
                resource: crate::http::safe_location(url).into_owned(),
                reason: format!(
                    "cached helper at {} failed its pinned sha256 check (expected {pin}, got {actual}) \
                     and re-provisioning failed: {error}; check the network and retry, or select \
                     a deliberate operator override",
                    executable.display()
                ),
            },
            None => FetchError::Source {
                resource: crate::http::safe_location(url).into_owned(),
                reason: format!(
                    "the pinned helper {} is not provisioned at {} and could not be acquired: \
                     {error}; check the network or the configured helper mirror and retry, or select \
                     a deliberate operator override",
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
) -> Result<ProvisionedHelper, FetchError> {
    if Instant::now() >= deadline {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Conda helper provisioning deadline expired",
        )
        .into());
    }
    let mut download =
        context.download_tool(url, pin, context.limits().download_bytes, deadline)?;
    download.file.as_file_mut().rewind()?;
    let expected = gripsack_process::Sha256Digest::parse(pin)?;
    let mut checked = checked::CheckedRead::new(
        download.file.as_file_mut(),
        expected,
        context.limits().download_bytes.get(),
        deadline,
    );
    gripsack_fs::atomic_copy_with_mode(&slot.directory, Path::new(slot.name), &mut checked, 0o755)?;
    Ok(provisioned(slot.executable.clone(), pin))
}

/// The cache check every use runs: owned regular bytes whose measured
/// sha256 equals the pin. Offline — no network is touched here.
fn warm(
    slot: &ToolSlot,
    pin: &str,
    context: &FetchContext,
    deadline: Instant,
) -> Result<Warm, FetchError> {
    use gripsack_fs::cap_std::fs::{MetadataExt, PermissionsExt};
    let file = match gripsack_fs::open_file_nofollow(&slot.directory, Path::new(slot.name)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Warm::Missing),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no preconditions.
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o7022 != 0
        || metadata.nlink() != 1
    {
        return Err(FetchError::Source {
            resource: slot.executable.display().to_string(),
            reason: "bridge cache must be an owned regular file, not writable by another user"
                .into(),
        });
    }
    if metadata.permissions().mode() & 0o111 != 0o111 {
        return Ok(Warm::Corrupt("non-executable mode".into()));
    }
    let actual = measured_hash(file.into_std(), context, deadline)?;
    Ok(if actual == pin {
        Warm::Ready
    } else {
        Warm::Corrupt(actual)
    })
}

/// Streaming sha256 of the cached executable, capped by the same
/// download bound that would govern its replacement.
fn measured_hash(
    file: std::fs::File,
    context: &FetchContext,
    deadline: Instant,
) -> Result<String, FetchError> {
    use sha2::Digest;
    use std::io::Read;
    let mut source = crate::spool::Limited::new(
        file,
        context.limits().download_bytes.get(),
        "Conda helper cache",
    );
    let mut hasher = sha2::Sha256::new();
    let mut chunk = [0; 64 * 1024];
    loop {
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "helper cache verification deadline expired",
            )
            .into());
        }
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

fn provisioned(path: PathBuf, pin: &str) -> ProvisionedHelper {
    ProvisionedHelper {
        path,
        pin: pin.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conda_bootstrap_executes_only_owned_pinned_bytes() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let (url, pin, _serving) = fixture(b"#!/bin/sh\nprintf bootstrap-proof\n");
        let context = FetchContext::default();
        let acquire = |origin: &str, deadline| {
            ensure_release(
                home.path(),
                context.provisioning(),
                VERSION,
                origin,
                &pin,
                deadline,
            )
        };
        let deadline = || Instant::now() + std::time::Duration::from_secs(5);
        let cold = acquire(&url, deadline()).unwrap();
        let run = || {
            let output = std::process::Command::new(&cold.path).output().unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, b"bootstrap-proof");
        };
        run();
        acquire("file:///missing-origin", deadline()).unwrap();
        run();
        std::fs::write(&cold.path, b"corrupt").unwrap();
        acquire(&url, deadline()).unwrap();
        run();
        std::fs::write(&cold.path, b"corrupt-offline").unwrap();
        assert!(acquire("file:///missing-origin", deadline()).is_err());
        assert!(!cold.path.exists());
        acquire(&url, deadline()).unwrap();
        run();
        std::fs::set_permissions(&cold.path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(acquire("file:///missing-origin", deadline()).is_err());
        acquire(&url, deadline()).unwrap();
        run();
        let foreign = home.path().join("foreign");
        std::fs::hard_link(&cold.path, &foreign).unwrap();
        assert!(acquire(&url, deadline()).is_err());
        std::fs::remove_file(&foreign).unwrap();
        std::fs::remove_file(&cold.path).unwrap();
        std::os::unix::fs::symlink("/bin/sh", &cold.path).unwrap();
        assert!(acquire(&url, deadline()).is_err());
        assert!(cold.path.symlink_metadata().unwrap().is_symlink());
        assert!(acquire(&url, Instant::now()).is_err());
        let root = gripsack_fs::open_or_create(home.path()).unwrap();
        let locks = gripsack_fs::open_dir_nofollow(&root, Path::new("locks")).unwrap();
        let _lock = gripsack_fs::FlockGuard::try_acquire_in(&locks, "conda-helper")
            .unwrap()
            .unwrap();
        let start = Instant::now();
        assert!(acquire(&url, start + std::time::Duration::from_millis(50)).is_err());
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn conda_namespace_aliases_and_writable_directories_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        for relative in [
            "tools".to_owned(),
            "locks".to_owned(),
            format!("tools/conda-helper-{VERSION}"),
        ] {
            let home = tempfile::tempdir().unwrap();
            let foreign = tempfile::tempdir().unwrap();
            let (url, pin, _serving) = fixture(b"#!/bin/sh\nexit 0\n");
            let alias = home.path().join(relative);
            std::fs::create_dir_all(alias.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(foreign.path(), &alias).unwrap();
            let acquire = || {
                ensure_release(
                    home.path(),
                    FetchContext::default().provisioning(),
                    VERSION,
                    &url,
                    &pin,
                    Instant::now() + std::time::Duration::from_secs(5),
                )
            };
            assert!(acquire().is_err());
            assert_eq!(std::fs::read_dir(foreign.path()).unwrap().count(), 0);
            std::fs::remove_file(&alias).unwrap();
            std::fs::create_dir(&alias).unwrap();
            std::fs::set_permissions(&alias, std::fs::Permissions::from_mode(0o777)).unwrap();
            assert!(acquire().is_err());
        }
    }

    const VERSION: &str = "9.9.9-test";

    /// `file://`-served fixture bytes plus their measured pin: the same
    /// shape a declared operator mirror serves in qualification.
    fn fixture(bytes: &[u8]) -> (String, String, tempfile::TempDir) {
        let serving = tempfile::tempdir().unwrap();
        let asset = serving.path().join("conda-helper-fixture");
        std::fs::write(&asset, bytes).unwrap();
        let pin = gripsack_store::hash::hex_sha256(bytes);
        (format!("file://{}", asset.display()), pin, serving)
    }
}
