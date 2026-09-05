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
    /// What gripsack last WROTE — or, when `preserved_drift` is set,
    /// what it OBSERVED (0029 §2: one field used to mean both, and
    /// observed user bytes became overwrite authority on the next
    /// apply).
    pub hash: String,
    /// Pre-take-over state of this destination (0015 §4) — carried
    /// forward across EVERY generation of the ownership epoch (0029
    /// §1): an origin is forgotten only by a successful restore or an
    /// explicit forget, never by an ordinary later apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior: Option<Prior>,
    /// True when gripsack preserved user bytes instead of deploying
    /// (0029 §2): such an entry authorizes NOTHING — apply re-evaluates
    /// the drift fresh, prune and rollback never touch the file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preserved_drift: bool,
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

/// Read a generation's manifest — the ONE strict boundary for
/// persisted generations (0027 §4). A generation is long-lived input
/// (disk faults, interrupted older releases, hand edits): parse is
/// not enough. Rejects: embedded number ≠ directory id, duplicate
/// destinations (case-folded — the E111 rule applies to history too),
/// malformed content hashes, store paths outside `$GRIPSACK_HOME/
/// store`. Prior blobs stay lazily read — restore-time errors already
/// abort the transaction.
pub fn read_manifest(home: &Path, generation: u64) -> io::Result<Generation> {
    let raw = std::fs::read(manifest_path(home, generation))?;
    let manifest: Generation =
        serde_json::from_slice(&raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    validate(&manifest, generation, home)?;
    Ok(manifest)
}

fn validate(manifest: &Generation, generation: u64, home: &Path) -> io::Result<()> {
    let invalid = |why: String| io::Error::new(io::ErrorKind::InvalidData, why);
    if manifest.number != generation {
        return Err(invalid(format!(
            "generation {generation}'s manifest claims number {} — directory and              identity disagree",
            manifest.number
        )));
    }
    let store_root = home.join(crate::paths::STORE_DIR);
    let mut seen_dests = std::collections::BTreeSet::new();
    for (name, state) in &manifest.modules {
        if !state.store_path.starts_with(&store_root) {
            return Err(invalid(format!(
                "module {name:?}: store path {} is outside $GRIPSACK_HOME/store",
                state.store_path.display()
            )));
        }
        for entry in &state.entries {
            // key: the destination for non-merge (E111 applies), and
            // (destination, module) for merge — several modules may own
            // separate blocks in one shared file (0029 §7: the 0027
            // validator rejected what merge legitimately allows)
            let key = if entry.mode == gripsack_ir::Ownership::Merge {
                format!("{}\u{1f}{}", entry.to.to_lowercase(), name)
            } else {
                entry.to.to_lowercase()
            };
            if !seen_dests.insert(key) {
                return Err(invalid(format!(
                    "destination {:?} appears twice in generation {generation}",
                    entry.to
                )));
            }
            if entry.hash.len() != 64 || !entry.hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(invalid(format!(
                    "module {name:?}: destination {:?} has a malformed content hash",
                    entry.to
                )));
            }
        }
    }
    Ok(())
}

/// All generation numbers on disk, ascending. Fail closed
/// (0027 §2): only a MISSING generations directory means "none" —
/// an enumeration or entry error is real (gc builds its deletion
/// set from this list; an empty list on error would collect the
/// active generation's store objects).
pub fn list(home: &Path) -> io::Result<Vec<u64>> {
    let dir = home.join(crate::paths::GENERATIONS_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut numbers = Vec::new();
    for entry in entries {
        let entry = entry?;
        if let Ok(n) = entry.file_name().to_string_lossy().parse() {
            numbers.push(n);
        }
    }
    numbers.sort_unstable();
    Ok(numbers)
}

/// The generation `current` points at, if any. Fail closed
/// (0026 §8): only NotFound means "no generations" — a permission
/// error, I/O failure, or a `current` link that parses to no
/// generation number are real errors, not absence (apply allocates
/// from this, gc protects it; misreading either is corruption).
pub fn current(home: &Path) -> io::Result<Option<u64>> {
    match std::fs::read_link(crate::paths::current_link(home)) {
        Ok(target) => {
            let n: u64 = target
                .file_name()
                .and_then(|n| n.to_string_lossy().parse().ok())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "{} does not point at a generation",
                            crate::paths::current_link(home).display()
                        ),
                    )
                })?;
            // the control plane and data plane must agree (0029 §10):
            // the link resolves to THIS home's generations/<n> — a
            // `current -> /tmp/42` is corruption, not generation 42
            let resolved = std::fs::canonicalize(crate::paths::current_link(home))?;
            let home_canon = std::fs::canonicalize(home)?;
            let expected = home_canon
                .join(crate::paths::GENERATIONS_DIR)
                .join(n.to_string());
            if resolved != expected {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "current resolves to {} — outside $GRIPSACK_HOME/generations",
                        resolved.display()
                    ),
                ));
            }
            Ok(Some(n))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Durable high-water mark: `generations/high-water` holds the highest
/// generation number ever allocated (0027 §9). Without it, gc of the
/// tip moves the on-disk maximum backward and IDs get reused — logs
/// and journal remnants would then name two different states "2".
pub fn allocate(home_path: &Path, home: &gripsack_fs::Dir) -> io::Result<u64> {
    let high_water = match home.read("generations/high-water") {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            let n = text.trim().parse::<u64>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "generations/high-water is corrupt — refusing to allocate",
                )
            })?;
            Some(n)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let next = [
        current(home_path)?,
        list(home_path)?.into_iter().max(),
        high_water,
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0)
        + 1;
    Ok(next)
}

/// Publish a complete generation as ONE object (0027 §8): manifest and
/// profile are staged under `generations/.staging-<N>` and renamed
/// into place with a no-clobber check — a failed apply never leaves a
/// half-populated `generations/N` visible to listings, allocation, or
/// rollback. A leftover staging dir from a crashed apply is not a
/// generation and is cleared first.
pub fn publish_generation(
    home: &gripsack_fs::Dir,
    generation: &Generation,
    profile: Option<&str>,
    home_path: &Path,
) -> io::Result<()> {
    // construction and load share the ONE validator (0029 §7): we
    // never publish what read_manifest would reject
    validate(generation, generation.number, home_path)?;
    let staging =
        Path::new(crate::paths::GENERATIONS_DIR).join(format!(".staging-{}", generation.number));
    let final_dir = Path::new(crate::paths::GENERATIONS_DIR).join(generation.number.to_string());
    if home.symlink_metadata(&final_dir).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "generation {} already exists — generations are immutable",
                generation.number
            ),
        ));
    }
    let _ = home.remove_dir_all(&staging);
    let json = serde_json::to_string_pretty(generation)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    gripsack_fs::atomic_write(home, &staging.join("manifest.json"), json.as_bytes())?;
    if let Some(profile) = profile {
        gripsack_fs::atomic_write(
            home,
            &staging.join("env").join("profile.sh"),
            profile.as_bytes(),
        )?;
    }
    // the high-water mark moves BEFORE the rename (0029 §9 ordering):
    // a failure after the rename must never leave a visible generation
    // the allocator doesn't know about
    gripsack_fs::atomic_write(
        home,
        Path::new("generations/high-water"),
        generation.number.to_string().as_bytes(),
    )?;
    gripsack_fs::fsync_dir(home, &staging)?;
    home.rename(&staging, home, &final_dir).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("publish generation {}: {e}", generation.number),
        )
    })?;
    gripsack_fs::fsync_dir(home, Path::new(crate::paths::GENERATIONS_DIR))
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

    fn mk_gen(home: &Path, n: u64) -> Generation {
        let mut modules = BTreeMap::new();
        modules.insert(
            "helix".to_string(),
            ModuleState {
                store_path: home.join("store/abc-helix"),
                entries: vec![DeployedEntry {
                    from: "config.toml".into(),
                    to: "~/.config/helix/config.toml".into(),
                    mode: Ownership::TrackedCopy,
                    vars: Default::default(),
                    hash: "d".repeat(64),
                    prior: None,
                    preserved_drift: false,
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
        write_manifest(&cap, &mk_gen(home, 1)).unwrap();
        write_manifest(&cap, &mk_gen(home, 2)).unwrap();
        assert_eq!(list(home).unwrap(), vec![1, 2]);
        let read = read_manifest(home, 1).unwrap();
        assert_eq!(read.modules["helix"].entries[0].hash, "d".repeat(64));
    }

    #[test]
    fn flip_and_current() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cap = gripsack_fs::open_or_create(home).unwrap();
        write_manifest(&cap, &mk_gen(home, 1)).unwrap();
        write_manifest(&cap, &mk_gen(home, 2)).unwrap();
        assert_eq!(current(home).unwrap(), None);
        flip(&cap, home, 1).unwrap();
        assert_eq!(current(home).unwrap(), Some(1));
        flip(&cap, home, 2).unwrap();
        assert_eq!(current(home).unwrap(), Some(2));
    }

    /// 0027 §4: the persisted-generation boundary rejects identity
    /// mismatch, duplicate destinations, malformed hashes, and store
    /// paths outside $GRIPSACK_HOME/store.
    #[test]
    fn manifests_are_strictly_validated() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cap = gripsack_fs::open_or_create(home).unwrap();

        // number ≠ directory: a manifest claiming 1, filed under 7
        write_manifest(&cap, &mk_gen(home, 1)).unwrap();
        let wrong = mk_gen(home, 1);
        // written by hand — publish's no-clobber guards directories,
        // this test is about CONTENT identity
        let bad = home.join("generations/7");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(
            bad.join("manifest.json"),
            serde_json::to_string_pretty(&wrong).unwrap(),
        )
        .unwrap();
        let err = read_manifest(home, 7).unwrap_err();
        assert!(err.to_string().contains("claims number"), "{err}");

        // duplicate destinations (case-folded)
        let mut dup = mk_gen(home, 3);
        let entry = dup.modules["helix"].entries[0].clone();
        dup.modules
            .get_mut("helix")
            .unwrap()
            .entries
            .push(DeployedEntry {
                to: "~/.config/HELIX/config.toml".into(),
                ..entry
            });
        let d = home.join("generations/3");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("manifest.json"),
            serde_json::to_string_pretty(&dup).unwrap(),
        )
        .unwrap();
        let err = read_manifest(home, 3).unwrap_err();
        assert!(err.to_string().contains("appears twice"), "{err}");

        // store path outside the store
        let mut outside = mk_gen(home, 4);
        outside.modules.get_mut("helix").unwrap().store_path = PathBuf::from("/etc/passwd");
        let d = home.join("generations/4");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("manifest.json"),
            serde_json::to_string_pretty(&outside).unwrap(),
        )
        .unwrap();
        let err = read_manifest(home, 4).unwrap_err();
        assert!(err.to_string().contains("outside"), "{err}");
    }

    /// 0027 §8/§9: publishing an existing generation fails no-clobber;
    /// allocation survives gc of the tip via the high-water mark.
    #[test]
    fn publish_is_atomic_and_allocation_monotonic() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cap = gripsack_fs::open_or_create(home).unwrap();
        publish_generation(&cap, &mk_gen(home, 1), Some("export A=1"), home).unwrap();
        assert!(home.join("generations/1/manifest.json").exists());
        assert!(home.join("generations/1/env/profile.sh").exists());
        // no staging residue
        assert!(!home.join("generations/.staging-1").exists());
        // no-clobber
        let err = publish_generation(&cap, &mk_gen(home, 1), None, home).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        // high-water: gc the tip, allocation still moves forward
        publish_generation(&cap, &mk_gen(home, 2), None, home).unwrap();
        publish_generation(&cap, &mk_gen(home, 3), None, home).unwrap();
        std::fs::remove_dir_all(home.join("generations/2")).unwrap();
        std::fs::remove_dir_all(home.join("generations/3")).unwrap();
        assert_eq!(allocate(home, &cap).unwrap(), 4);
    }
}
