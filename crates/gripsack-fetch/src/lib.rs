//! Fetcher plugin discovery (plan/0002 §4) — the runtime-pluggable
//! transport seam.
//!
//! ```text
//! module says {"kind": "plugin", "name": "artifactory", ...}
//!     ▼
//! find gripfetch-artifactory on $PATH
//!   (or an explicit path from [fetchers.artifactory] in env.toml)
//!     ▼
//! the core drives it over NDJSON/stdio and hash-verifies
//! every returned byte against the lockfile before it enters the store
//! ```
//!
//! Discovery only, for now; the protocol host lands with the executor.

pub mod fetch;
pub mod host;
pub(crate) mod http;
pub mod plugins;
pub mod resolve;
pub mod throttle;

pub use fetch::{FetchError, fetch, fetch_with_locked, payload_hash};
pub use host::{AssetTarget, DENO_RELEASE, PIXI_RELEASE, resolve as resolve_host_asset};
pub use resolve::{
    ResolvedRelease, SelfRelease, resolve_brew, resolve_latest, resolve_self_release,
};

use std::path::PathBuf;

pub const FETCHER_PREFIX: &str = "gripfetch-";

/// The executable name for a fetcher plugin, e.g. `artifactory` →
/// `gripfetch-artifactory`.
pub fn fetcher_exe(name: &str) -> String {
    format!("{FETCHER_PREFIX}{name}")
}

/// Find a fetcher on `PATH`. Returns the full path to the executable.
/// Explicit executable paths registered from env.toml (`[fetchers.x]
/// path = …` or `plugin = …`), consulted before the store and PATH —
/// the offline/air-gapped route (enterprise review).
static REGISTERED: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<String, PathBuf>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

pub fn register_fetcher_path(name: &str, path: PathBuf) {
    REGISTERED
        .lock()
        .expect("fetcher registry")
        .insert(name.to_string(), path);
}

pub fn find_fetcher(name: &str) -> Option<PathBuf> {
    let exe = fetcher_exe(name);
    if let Some(path) = REGISTERED.lock().expect("fetcher registry").get(name)
        && path.is_file()
    {
        return Some(path.clone());
    }
    // the managed plugin store wins (declared in env.toml, provisioned
    // at eval — 0012 §move-2); PATH is the unmanaged fallback
    let store = plugins::PluginStore::new(&gripsack_store::gripsack_home());
    if let Some(bin) = store.current_binary(&exe) {
        return Some(bin);
    }
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(&exe))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn exe_naming() {
        assert_eq!(fetcher_exe("artifactory"), "gripfetch-artifactory");
    }

    #[cfg(unix)]
    #[test]
    fn discovers_executable_on_path() {
        // Single test mutating PATH to avoid cross-test races.
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("gripfetch-internal");
        fs::write(&exe, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let non_exe = dir.path().join("gripfetch-notexec");
        fs::write(&non_exe, "#!/bin/sh\n").unwrap();

        let original = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", dir.path()) };
        assert_eq!(find_fetcher("internal"), Some(exe));
        assert_eq!(find_fetcher("notexec"), None);
        assert_eq!(find_fetcher("absent"), None);
        match original {
            Some(v) => unsafe { std::env::set_var("PATH", v) },
            None => unsafe { std::env::remove_var("PATH") },
        }
    }
}
