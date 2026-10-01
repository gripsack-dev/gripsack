//! Lazy provisioning of the optional `gripsack-conda` helper (A3): the
//! core never links Rattler; the helper binary is provisioned on first
//! use from this release's compiled-in per-platform pins, downloaded
//! through the bounded FetchContext and verified against its SHA-256
//! pin before it is ever executed. An operator mirror override
//! (`GRIPSACK_CONDA_HELPER_MIRROR`, a directory or base URL containing
//! `<platform>/gripsack-conda`) replaces only the origin, never the
//! pin. Until a release publishes helper assets the pin table is empty
//! and provisioning is an explicit error — no invented hashes, no
//! silent fallback to ambient binaries.

use gripsack_fetch::FetchContext;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A provisioned helper: exact pinned bytes in a durable slot.
pub struct ProvisionedHelper {
    pub path: PathBuf,
    pub pin: String,
}

/// One platform's compiled-in helper identity. Filled by the release
/// lane from MEASURED asset hashes only.
struct HelperPin {
    platform: &'static str,
    sha256: &'static str,
}

/// The pin table for this release. Empty until helper assets are
/// published; an empty table is an explicit provisioning error.
const HELPER_PINS: &[HelperPin] = &[];

fn host_platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        ("macos", "aarch64") => "macos-aarch64",
        _ => "unsupported",
    }
}

/// Provisioning may wait on a competing slot lock and one bounded
/// download; the download itself keeps the fetch context's own
/// per-request timeouts. This budget covers the wait, not the wire.
const PROVISIONING_BUDGET: Duration = Duration::from_secs(900);

/// Provision the pinned helper for this platform, or explain exactly
/// which prerequisite is missing.
pub fn ensure(home: &Path, context: &FetchContext) -> Result<ProvisionedHelper, io::Error> {
    let platform = host_platform();
    let pin = HELPER_PINS
        .iter()
        .find(|pin| pin.platform == platform)
        .ok_or_else(|| {
            io::Error::other(format!(
                "no pinned gripsack-conda helper for {platform} in this release; \
                 until the release publishes per-platform helper assets, build \
                 crates/gripsack-conda with --features helper and set GRIPSACK_CONDA_HELPER"
            ))
        })?;
    let deadline = Instant::now() + PROVISIONING_BUDGET;
    let slot = home.join("tools").join("conda-helper").join(pin.sha256);
    let executable = slot.join("gripsack-conda");
    if executable.is_file() {
        return Ok(ProvisionedHelper {
            path: executable,
            pin: pin.sha256.into(),
        });
    }
    let origin = std::env::var("GRIPSACK_CONDA_HELPER_MIRROR")
        .unwrap_or_else(|_| "https://github.com/gripsack-dev/gripsack/releases".into());
    let url = format!(
        "{}/{}/gripsack-conda",
        origin.trim_end_matches('/'),
        platform
    );
    let url = url
        .strip_prefix('/')
        .map(|path| format!("file://{path}"))
        .unwrap_or(url);
    let download = context
        .download_verified(&url, pin.sha256)
        .map_err(|error| io::Error::other(format!("helper download: {error}")))?;
    std::fs::create_dir_all(&slot)?;
    let staged = slot.join(".staging");
    download
        .file
        .persist(&staged)
        .map_err(|error| io::Error::other(format!("helper spool: {}", error.error)))?;
    let mut permissions = std::fs::metadata(&staged)?.permissions();
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o755);
    }
    std::fs::set_permissions(&staged, permissions)?;
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "helper provisioning deadline expired",
        ));
    }
    std::fs::rename(&staged, &executable).or_else(|error| {
        if executable.is_file() {
            Ok(())
        } else {
            Err(error)
        }
    })?;
    Ok(ProvisionedHelper {
        path: executable,
        pin: pin.sha256.into(),
    })
}
