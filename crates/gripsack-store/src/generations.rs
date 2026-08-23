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
use gripsack_ir::Ownership;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// One deployed file: where it went, how, and its canonical hash at
/// deploy time (drift detection compares against this — 0008 §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeployedEntry {
    pub from: String,
    pub to: String,
    pub mode: Ownership,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleState {
    pub store_path: PathBuf,
    #[serde(default)]
    pub entries: Vec<DeployedEntry>,
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

/// Flip `current` to a generation — the single indivisible activation
/// operation (0001 §9.2).
pub fn flip(home: &Path, generation: u64) -> io::Result<()> {
    fs::symlink_replace(
        &crate::paths::current_link(home),
        &crate::paths::generation_dir(home, generation),
    )
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
                    hash: "deadbeef".into(),
                }],
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
