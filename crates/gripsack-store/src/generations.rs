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

/// Write a generation's manifest atomically, relative to the home
/// capability (plan/0021): `generations/<N>/manifest.json` can never
/// be redirected by a swapped path component.
pub fn write_manifest(home: &gripsack_fs::Dir, generation: &Generation) -> io::Result<()> {
    // generations are immutable history (0026 §3): an existing
    // generation number is a hard invariant failure, never an
    // overwrite target (the pre-0.23 current+1 allocator could reuse
    // one after a rollback)
    let gen_rel = Path::new(crate::paths::GENERATIONS_DIR).join(generation.number.to_string());
    if home.symlink_metadata(&gen_rel).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "generation {} already exists — generations are immutable",
                generation.number
            ),
        ));
    }
    let json = serde_json::to_string_pretty(generation)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    gripsack_fs::atomic_write(
        home,
        &Path::new(crate::paths::GENERATIONS_DIR)
            .join(generation.number.to_string())
            .join("manifest.json"),
        json.as_bytes(),
    )
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

/// The generation `current` points at, if any. Fail closed
/// (0026 §8): only NotFound means "no generations" — a permission
/// error, I/O failure, or a `current` link that parses to no
/// generation number are real errors, not absence (apply allocates
/// from this, gc protects it; misreading either is corruption).
pub fn current(home: &Path) -> io::Result<Option<u64>> {
    match std::fs::read_link(crate::paths::current_link(home)) {
        Ok(target) => target
            .file_name()
            .and_then(|n| n.to_string_lossy().parse().ok())
            .map(Some)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{} does not point at a generation",
                        crate::paths::current_link(home).display()
                    ),
                )
            }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// [`current`] through the home capability (plan/0021): the link is
/// read relative to the `Dir`, never re-resolved by string.
pub fn current_in(home: &gripsack_fs::Dir) -> std::io::Result<Option<u64>> {
    match home.read_link_contents("current") {
        Ok(target) => target
            .file_name()
            .and_then(|n| n.to_string_lossy().parse().ok())
            .map(Some)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "current does not point at a generation".to_string(),
                )
            }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        // recovery reads commit evidence through this: a permission
        // or I/O error is not "no generations" (0025 §F)
        Err(e) => Err(e),
    }
}

/// Flip `current` to a generation — the single indivisible activation
/// operation (0001 §9.2), pinned to the home capability (plan/0021).
/// `home_path` exists only to compose the link TARGET: `current`
/// records the absolute generation dir so readers resolve it from
/// any cwd. The target must exist first: a `current` pointing at
/// nothing reads as "no generations" everywhere downstream
/// (list/current swallow that as None) while looking deployed to
/// anything that inspects the link.
pub fn flip(home: &gripsack_fs::Dir, home_path: &Path, generation: u64) -> io::Result<()> {
    let rel = Path::new(crate::paths::GENERATIONS_DIR).join(generation.to_string());
    let dir = home_path.join(&rel);
    if !home.metadata(&rel).is_ok_and(|m| m.is_dir()) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "cannot activate generation {generation}: {} is missing",
                dir.display()
            ),
        ));
    }
    gripsack_fs::symlink_replace(home, Path::new("current"), &dir)
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
        let cap = gripsack_fs::open_or_create(home).unwrap();
        write_manifest(&cap, &mk_gen(1)).unwrap();
        write_manifest(&cap, &mk_gen(2)).unwrap();
        assert_eq!(list(home), vec![1, 2]);
        let read = read_manifest(home, 1).unwrap();
        assert_eq!(read.modules["helix"].entries[0].hash, "deadbeef");
    }

    #[test]
    fn flip_and_current() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cap = gripsack_fs::open_or_create(home).unwrap();
        write_manifest(&cap, &mk_gen(1)).unwrap();
        write_manifest(&cap, &mk_gen(2)).unwrap();
        assert_eq!(current(home).unwrap(), None);
        flip(&cap, home, 1).unwrap();
        assert_eq!(current(home).unwrap(), Some(1));
        flip(&cap, home, 2).unwrap();
        assert_eq!(current(home).unwrap(), Some(2));
    }
}
