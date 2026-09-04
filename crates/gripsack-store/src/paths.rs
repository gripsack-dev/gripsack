//! Store paths (plan/0001 §3.4, hybrid per plan/0014).
//!
//! A store path is `<gripsack-home>/store/<input-hash>-<name>` where the
//! input hash covers the resolved module plan (fetch spec + pinned refs +
//! build recipe + dependency hashes). Store paths are immutable; the same
//! resolved inputs produce the same path on any machine of the same
//! platform — that is what makes store sharing a trivial later feature.

use std::path::{Path, PathBuf};

pub const STORE_DIR: &str = "store";
pub const GENERATIONS_DIR: &str = "generations";
/// Length of the hex input hash prefix in store path names.
pub const HASH_LEN: usize = 16;

/// Hex sha256 prefix of the canonical serialized module plan. `canonical`
/// must be a stable serialization (serde struct field order is).
pub fn input_hash(canonical: &str) -> String {
    crate::hash::hex_sha256(canonical.as_bytes())[..HASH_LEN * 2].to_string()
}

/// The store path for a module with the given resolved plan.
pub fn store_path(home: &Path, name: &str, canonical: &str) -> PathBuf {
    home.join(STORE_DIR)
        .join(format!("{}-{name}", input_hash(canonical)))
}

/// Content-addressed store path (0014): the key IS the canonical tree
/// hash of the published payload — the name is the expectation, so a
/// path's presence is proof of content. Same width as input hashes.
/// `tree256` arrives from lockfiles and manifests (disk state): a
/// short or malformed value must miss the store, never panic.
pub fn content_path(home: &Path, name: &str, tree256: &str) -> PathBuf {
    let prefix = tree256.get(..HASH_LEN * 2).unwrap_or(tree256);
    home.join(STORE_DIR).join(format!("{prefix}-{name}"))
}

/// Base directory for everything gripsack owns: store, generations, and
/// the `current` symlink. `$GRIPSACK_HOME` wins, then
/// `$XDG_DATA_HOME/gripsack`, then `~/.local/share/gripsack`.
pub fn gripsack_home() -> PathBuf {
    // empty-string vars count as unset: honoring `GRIPSACK_HOME=""`
    // would make every store path relative to the CWD
    if let Some(dir) = std::env::var_os("GRIPSACK_HOME")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    if let Some(data) = std::env::var_os("XDG_DATA_HOME")
        && !data.is_empty()
    {
        return PathBuf::from(data).join("gripsack");
    }
    // no HOME and no override: there is no defensible location, and
    // inventing one (cwd, /tmp) would scatter the store. Say it.
    std::env::var_os("HOME").map_or_else(
        || panic!("grip needs HOME, GRIPSACK_HOME, or XDG_DATA_HOME to place its store"),
        |home| PathBuf::from(home).join(".local/share/gripsack"),
    )
}

/// `~/x` → `$HOME/x`; anything else verbatim. The one expansion rule
/// shared by deploy, rollback, and why-owns — it lives with the other
/// path rules so the CLI and the executor cannot drift on it.
pub fn expand_home(to: &str) -> PathBuf {
    if let Some(rest) = to.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(to)
}

/// Where a prior blob lives: `$GRIPSACK_HOME/prior/<sha256>`.
pub fn prior_blob_path(home: &Path, sha: &str) -> PathBuf {
    home.join("prior").join(sha)
}

/// The `current` symlink — flipping it IS activation (0001 §9.2).
pub fn current_link(home: &Path) -> PathBuf {
    home.join("current")
}

/// Directory of one generation's profile tree.
pub fn generation_dir(home: &Path, generation: u64) -> PathBuf {
    home.join(GENERATIONS_DIR).join(generation.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_input_sensitive() {
        assert_eq!(input_hash("plan-a"), input_hash("plan-a"));
        assert_ne!(input_hash("plan-a"), input_hash("plan-b"));
        assert_eq!(input_hash("plan-a").len(), HASH_LEN * 2);
    }

    #[test]
    fn store_path_format() {
        let p = store_path(Path::new("/gs"), "helix", "plan-a");
        let name = p.file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with("-helix"));
        assert_eq!(name.len(), HASH_LEN * 2 + 1 + "helix".len());
        assert_eq!(p.parent().unwrap(), Path::new("/gs/store"));
    }

    #[test]
    fn home_resolution() {
        // Single test mutating env to avoid cross-test races.
        unsafe { std::env::set_var("GRIPSACK_HOME", "/tmp/gs-explicit") };
        assert_eq!(gripsack_home(), Path::new("/tmp/gs-explicit"));
        unsafe { std::env::remove_var("GRIPSACK_HOME") };
        unsafe { std::env::set_var("XDG_DATA_HOME", "/tmp/gs-xdg") };
        assert_eq!(gripsack_home(), Path::new("/tmp/gs-xdg/gripsack"));
        unsafe { std::env::remove_var("XDG_DATA_HOME") };
        unsafe { std::env::set_var("HOME", "/tmp/gs-home") };
        assert_eq!(
            gripsack_home(),
            Path::new("/tmp/gs-home/.local/share/gripsack")
        );
    }

    #[test]
    fn generation_layout() {
        let home = Path::new("/gs");
        assert_eq!(current_link(home), Path::new("/gs/current"));
        assert_eq!(generation_dir(home, 42), Path::new("/gs/generations/42"));
    }
}
