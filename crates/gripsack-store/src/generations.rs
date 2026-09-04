//! Generations on disk: the manifest of what each generation deployed,
//! and the flip (plan/0001 §3.5, 0008 §3).
//!
//! ```text
//! generations/
//! ├── 1/manifest.json     {number, modules: {name: {store_path, entries[]}}}
//! ├── 2/manifest.json
//! └── ...
//! current -> generations/2
//! ```

use crate::fs;
use gripsack_ir::{EnvVar, Ownership};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// What a destination was before gripsack took it over (0015 §4) —
/// recorded on every take-over, restored by rollback and prune instead
/// of deletion. "your original files have been restored."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prior {
    pub kind: PriorKind,
    /// File: sha256 of the bytes in the prior blob store.
    /// Symlink: the link target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Original permission bits (unix) — a restore is faithful or it
    /// isn't a restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriorKind {
    File,
    Symlink,
}

/// One deployed file: where it went, how, and its canonical hash at
/// deploy time (drift detection compares against this — 0008 §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeployedEntry {
    pub from: String,
    pub to: String,
    pub mode: Ownership,
    /// Template vars at deploy time — rollback re-renders with these.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub vars: std::collections::BTreeMap<String, String>,
    pub hash: String,
    /// Pre-take-over state of this destination (0015 §4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior: Option<Prior>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleState {
    pub store_path: PathBuf,
    #[serde(default)]
    pub entries: Vec<DeployedEntry>,
    /// Environment contributions, replayed into the shell profile at
    /// activation and rollback (0001 §3.10).
    #[serde(default)]
    pub env: Vec<EnvVar>,
    /// Content identity of the published tree (0014): present for
    /// content-addressed modules — store verify compares the live tree
    /// against this, no lockfile lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree256: Option<String>,
}

/// A generation: an immutable record of one profile state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Generation {
    pub number: u64,
    pub modules: BTreeMap<String, ModuleState>,
}

fn manifest_path(home: &Path, generation: u64) -> PathBuf {
    crate::paths::generation_dir(home, generation).join("manifest.json")
}

/// Write a generation's manifest atomically.
pub fn write_manifest(home: &Path, generation: &Generation) -> io::Result<()> {
    let json = serde_json::to_string_pretty(generation)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::atomic_write(&manifest_path(home, generation.number), json.as_bytes())
}

/// Read a generation's manifest.
pub fn read_manifest(home: &Path, generation: u64) -> io::Result<Generation> {
    let raw = std::fs::read(manifest_path(home, generation))?;
    serde_json::from_slice(&raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// All generation numbers on disk, ascending.
pub fn list(home: &Path) -> Vec<u64> {
    let dir = home.join(crate::paths::GENERATIONS_DIR);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut numbers: Vec<u64> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().parse().ok())
        .collect();
    numbers.sort_unstable();
    numbers
}

/// The generation `current` points at, if any.
pub fn current(home: &Path) -> Option<u64> {
    std::fs::read_link(crate::paths::current_link(home))
        .ok()?
        .file_name()?
        .to_string_lossy()
        .parse()
        .ok()
}

/// [`current`] through the home capability (plan/0021): the link is
/// read relative to the `Dir`, never re-resolved by string.
pub fn current_in(home: &gripsack_fs::Dir) -> Option<u64> {
    home.read_link_contents("current")
        .ok()?
        .file_name()?
        .to_string_lossy()
        .parse()
        .ok()
}

/// Flip `current` to a generation — the single indivisible activation
/// operation (0001 §9.2). The target must exist first: a `current`
/// pointing at nothing reads as "no generations" everywhere
/// downstream (list/current swallow that as None) while looking
/// deployed to anything that inspects the link.
pub fn flip(home: &Path, generation: u64) -> io::Result<()> {
    let dir = crate::paths::generation_dir(home, generation);
    if !dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "cannot activate generation {generation}: {} is missing",
                dir.display()
            ),
        ));
    }
    fs::symlink_replace(&crate::paths::current_link(home), &dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_gen(n: u64) -> Generation {
        let mut modules = BTreeMap::new();
        modules.insert(
            "helix".to_string(),
            ModuleState {
                store_path: PathBuf::from("/store/abc-helix"),
                entries: vec![DeployedEntry {
                    from: "config.toml".into(),
                    to: "~/.config/helix/config.toml".into(),
                    mode: Ownership::TrackedCopy,
                    vars: Default::default(),
                    hash: "deadbeef".into(),
                    prior: None,
                }],
                env: vec![],
                tree256: None,
            },
        );
        Generation { number: n, modules }
    }

    #[test]
    fn manifest_roundtrip_and_listing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_manifest(home, &mk_gen(1)).unwrap();
        write_manifest(home, &mk_gen(2)).unwrap();
        assert_eq!(list(home), vec![1, 2]);
        let read = read_manifest(home, 1).unwrap();
        assert_eq!(read.modules["helix"].entries[0].hash, "deadbeef");
    }

    #[test]
    fn flip_and_current() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_manifest(home, &mk_gen(1)).unwrap();
        write_manifest(home, &mk_gen(2)).unwrap();
        assert_eq!(current(home), None);
        flip(home, 1).unwrap();
        assert_eq!(current(home), Some(1));
        flip(home, 2).unwrap();
        assert_eq!(current(home), Some(2));
    }
}
