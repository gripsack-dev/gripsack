//! `gc` and `why-owns` — the store's hygiene commands (0001 §gc).
//!
//! Generations pin store paths; anything no generation references is
//! collectable. `keep_generations` bounds how many generations live
//! (user config `~/.config/gripsack/config.toml`); the current
//! generation is never touched.

use crate::ctx::ExecError;
use gripsack_store as store;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct GcReport {
    pub generations_removed: Vec<u64>,
    pub store_removed: Vec<PathBuf>,
    pub bytes_freed: u64,
}

/// Collect unreferenced store paths, and generations beyond `keep`
/// (never the current one). Generation pruning happens first — paths
/// only referenced by a pruned generation become collectable too.
/// `dry_run` reports without deleting (0003: plan-before-apply
/// applies to the destructive commands too, N6).
pub fn gc(home: &Path, keep: Option<u32>, dry_run: bool) -> Result<GcReport, ExecError> {
    let mut report = GcReport::default();
    // fail closed (0027 §2): enumeration errors propagate — gc's
    // deletion set derives from this inventory, and "cannot read"
    // must never read as "nothing referenced"
    let generations = store::list_generations(home)?;
    let current = store::current_generation(home)?;
    // the active generation must be IN the inventory before any plan
    // is computed — a current without a directory is corruption
    if let Some(c) = current
        && !generations.contains(&c)
    {
        return Err(ExecError::Step {
            module: "*".into(),
            step: "gc".into(),
            detail: format!(
                "current generation {c} has no directory on disk — refusing to collect"
            ),
        });
    }

    // what WOULD be pruned (dry-run must preview the post-prune state,
    // or it under-reports collectable paths)
    let mut pruned = std::collections::BTreeSet::new();
    if let Some(keep) = keep {
        let keep = keep as usize;
        if generations.len() > keep {
            let excess = generations.len() - keep;
            for n in &generations[..excess] {
                if Some(*n) == current {
                    continue; // never the active one — keep one extra instead
                }
                pruned.insert(*n);
                report.generations_removed.push(*n);
            }
        }
    }

    let mut referenced = std::collections::BTreeSet::new();
    for n in &generations {
        if pruned.contains(n) {
            continue;
        }
        // fail CLOSED: an unparseable manifest must abort gc — dropping
        // its pins would collect referenced store paths and leave
        // dangling symlinks across the user's home (review finding G)
        let manifest = store::read_manifest(home, *n).map_err(|e| ExecError::Step {
            module: format!("generation {n}"),
            step: "gc".into(),
            detail: format!("manifest is corrupt — refusing to collect: {e}"),
        })?;
        for state in manifest.modules.values() {
            referenced.insert(state.store_path.clone());
        }
        // 0015 §4: generations pin prior blobs the same way — a prior
        // is restorable exactly while its generation lives
        for state in manifest.modules.values() {
            for entry in &state.entries {
                if let Some(prior) = &entry.prior
                    && prior.kind == store::PriorKind::File
                    && let Some(sha) = &prior.content
                {
                    referenced.insert(store::prior_blob_path(home, sha));
                }
            }
        }
    }
    if !dry_run {
        for n in &pruned {
            std::fs::remove_dir_all(store::generation_dir(home, *n))?;
        }
    }

    let store_dir = home.join(store::STORE_DIR);
    if store_dir.is_dir() {
        for entry in std::fs::read_dir(&store_dir)? {
            let path = entry?.path();
            if !referenced.contains(&path) {
                report.bytes_freed += dir_size(&path)?;
                if !dry_run {
                    std::fs::remove_dir_all(&path)?;
                }
                report.store_removed.push(path);
            }
        }
    }
    // prior blobs (0015 §4): same reachability rule, flat dir of files
    let prior_dir = home.join("prior");
    if prior_dir.is_dir() {
        for entry in std::fs::read_dir(&prior_dir)? {
            let path = entry?.path();
            if !referenced.contains(&path) {
                report.bytes_freed += dir_size(&path)?;
                if !dry_run {
                    std::fs::remove_file(&path)?;
                }
                report.store_removed.push(path);
            }
        }
    }
    Ok(report)
}

fn dir_size(path: &Path) -> io::Result<u64> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_file() {
        return Ok(meta.len());
    }
    if !meta.is_dir() {
        return Ok(0); // symlink/fifo/socket: size is not content
    }
    let mut total = 0;
    for entry in std::fs::read_dir(path)? {
        total += dir_size(&entry?.path())?;
    }
    Ok(total)
}

/// Which module owns a deployed path, per the current generation's
/// manifest. Matches the declared `to` or its absolute expansion.
pub fn why_owns(
    home: &Path,
    path: &str,
) -> Result<Option<(String, store::DeployedEntry)>, ExecError> {
    let Some(n) = store::current_generation(home)? else {
        return Ok(None);
    };
    let manifest = store::read_manifest(home, n)?;
    for (name, state) in &manifest.modules {
        for entry in &state.entries {
            if entry.to == path || gripsack_store::expand_home(&entry.to).to_string_lossy() == path
            {
                return Ok(Some((name.clone(), entry.clone())));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_ir::Ownership;
    use std::fs;

    fn mk_gen(n: u64, store_path: PathBuf) -> store::Generation {
        let mut modules = std::collections::BTreeMap::new();
        modules.insert(
            "m".to_string(),
            store::ModuleState {
                store_path,
                entries: vec![store::DeployedEntry {
                    from: "a".into(),
                    to: "~/.config/m/a".into(),
                    mode: Ownership::TrackedCopy,
                    vars: Default::default(),
                    hash: "a".repeat(64),
                    prior: None,
                    preserved_drift: false,
                }],
                env: vec![],
                tree256: None,
            },
        );
        store::Generation { number: n, modules }
    }

    fn setup() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        for (n, tag) in [(1, "aaa"), (2, "bbb"), (3, "ccc")] {
            let sp = home.join("store").join(format!("{tag}-m"));
            fs::create_dir_all(&sp).unwrap();
            fs::write(sp.join("payload"), format!("gen {n}")).unwrap();
            store::write_manifest(&gripsack_fs::open_or_create(home).unwrap(), &mk_gen(n, sp))
                .unwrap();
        }
        // an orphan path no manifest references
        let orphan = home.join("store").join("zzz-orphan");
        fs::create_dir_all(&orphan).unwrap();
        fs::write(orphan.join("payload"), b"old").unwrap();
        store::flip(&gripsack_fs::open_or_create(home).unwrap(), home, 3).unwrap();
        dir
    }

    #[test]
    fn collects_only_unreferenced_store_paths() {
        let dir = setup();
        let home = dir.path();
        let report = gc(home, None, false).unwrap();
        assert_eq!(report.store_removed.len(), 1);
        assert!(report.store_removed[0].ends_with("zzz-orphan"));
        assert!(home.join("store/aaa-m").exists());
        assert!(report.bytes_freed > 0);
    }

    #[test]
    fn keep_generations_prunes_oldest_and_their_paths() {
        let dir = setup();
        let home = dir.path();
        let report = gc(home, Some(2), false).unwrap();
        assert_eq!(report.generations_removed, vec![1]);
        assert!(!store::generation_dir(home, 1).exists());
        assert!(store::generation_dir(home, 3).exists());
        // gen 1's store path is unreferenced now → collected; gen 2's stays
        assert!(report.store_removed.iter().any(|p| p.ends_with("aaa-m")));
        assert!(
            report
                .store_removed
                .iter()
                .any(|p| p.ends_with("zzz-orphan"))
        );
        assert!(home.join("store/bbb-m").exists());
    }

    #[test]
    fn never_prunes_the_current_generation() {
        let dir = setup();
        let home = dir.path();
        let report = gc(home, Some(1), false).unwrap();
        assert!(store::generation_dir(home, 3).exists());
        assert!(!report.generations_removed.contains(&3));
        assert!(home.join("store/ccc-m").exists());
    }

    #[test]
    fn why_owns_finds_the_owner() {
        let dir = setup();
        let home = dir.path();
        let (name, entry) = why_owns(home, "~/.config/m/a").unwrap().unwrap();
        assert_eq!(name, "m");
        assert_eq!(entry.mode, Ownership::TrackedCopy);
        let absolute = gripsack_store::expand_home("~/.config/m/a");
        assert!(
            why_owns(home, &absolute.to_string_lossy())
                .unwrap()
                .is_some()
        );
        assert!(why_owns(home, "/nope").unwrap().is_none());
    }
}
