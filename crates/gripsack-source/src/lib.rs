//! Sourcerer discovery (plan/0002 §4).
//!
//! A sourcerer is an executable named `gripsource-<name>` on `PATH` — the
//! git remote-helper model. It speaks NDJSON over stdio and materializes
//! a pinned fetch into a directory; the core verifies the returned bytes
//! against the lockfile before anything enters the store. This module
//! only discovers them; the protocol host lands with the executor.

use std::path::PathBuf;

pub const SOURCERER_PREFIX: &str = "gripsource-";

/// The executable name for a sourcerer plugin, e.g. `artifactory` →
/// `gripsource-artifactory`.
pub fn sourcerer_exe(name: &str) -> String {
    format!("{SOURCERER_PREFIX}{name}")
}

/// Find a sourcerer on `PATH`. Returns the full path to the executable.
pub fn find_sourcerer(name: &str) -> Option<PathBuf> {
    let exe = sourcerer_exe(name);
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
        assert_eq!(sourcerer_exe("artifactory"), "gripsource-artifactory");
    }

    #[cfg(unix)]
    #[test]
    fn discovers_executable_on_path() {
        // Single test mutating PATH to avoid cross-test races.
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("gripsource-internal");
        fs::write(&exe, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let non_exe = dir.path().join("gripsource-notexec");
        fs::write(&non_exe, "#!/bin/sh\n").unwrap();

        let original = std::env::var_os("PATH");
        std::env::set_var("PATH", dir.path());
        assert_eq!(find_sourcerer("internal"), Some(exe));
        assert_eq!(find_sourcerer("notexec"), None);
        assert_eq!(find_sourcerer("absent"), None);
        match original {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
    }
}
