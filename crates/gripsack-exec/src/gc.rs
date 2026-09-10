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
///
/// Requires a [`LifecycleSession`] (0045 F4): gc deletes store paths
/// an in-flight apply may have published but not yet flipped, so the
/// serialization contract is part of the signature, not a caller
/// convention.
pub fn gc(
    session: &crate::LifecycleSession,
    keep: Option<u32>,
    dry_run: bool,
) -> Result<GcReport, ExecError> {
    let home = session.home();
    let mut report = GcReport::default();
    // fail closed (0027 §2): enumeration errors propagate — gc's
    // deletion set derives from this inventory, and "cannot read"
    // must never read as "nothing referenced". An unreadable journal
    // fails the same way (pending_recovery errors).
    let home_cap = gripsack_fs::open_or_create(home)?;
    let pending = store::journal::pending_recovery(&home_cap)?;
    let generations = store::list_generations(home)?;
    let current = store::current_generation(home)?;

    // Admission as a total function (0045 F3, 0046): the kernel's
    // biconditionals mean a forgotten check is a type-level impossibility.
    use gripsack_policy::retention::GcAdmission;
    match gripsack_policy::retention::admit_gc(pending.is_some(), current, &generations) {
        // recovery state is load-bearing: a journaled prior blob may
        // be referenced by NO retained manifest, so collecting before
        // reconcile would destroy the bytes recovery needs. Refused
        // dry-run included: the deletion set is unsound while recovery
        // is pending, and a preview that lies is worse than no preview.
        GcAdmission::RecoveryPending => {
            return Err(ExecError::Step {
                module: "*".into(),
                step: "gc".into(),
                detail: format!(
                    "recovery state is pending ({}) — run `grip apply` or \
                     `grip rollback` to reconcile before collecting; nothing was deleted",
                    pending.expect("RecoveryPending implies pending state")
                ),
            });
        }
        // the active generation must be IN the inventory before any
        // plan is computed — a current without a directory is corruption
        GcAdmission::CorruptCurrent => {
            return Err(ExecError::Step {
                module: "*".into(),
                step: "gc".into(),
                detail: format!(
                    "current generation {} has no directory on disk — refusing to collect",
                    current.expect("CorruptCurrent implies a current generation")
                ),
            });
        }
        GcAdmission::Admitted => {}
    }

    // what WOULD be pruned (dry-run must preview the post-prune state,
    // or it under-reports collectable paths). The kernel (0046): pruned
    // generations come from the inventory and never name the current
    // generation — proved, not reviewed.
    let pruned: std::collections::BTreeSet<u64> =
        gripsack_policy::retention::plan_prune(&generations, current, keep)
            .into_iter()
            .collect();
    report.generations_removed = pruned.iter().copied().collect();

    // Root computation (the trusted adapter around the proven deletion
    // kernel): retained generations pin their store paths, their build
    // closures, and their prior blobs. Completeness of these roots at
    // production time is a separate obligation (handoff §5.3).
    let mut referenced: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for n in &generations {
        // fail CLOSED: an unparseable manifest must abort gc — dropping
        // its pins would collect referenced store paths and leave
        // dangling symlinks across the user's home (review finding G)
        let manifest = store::read_manifest(home, *n).map_err(|e| ExecError::Step {
            module: format!("generation {n}"),
            step: "gc".into(),
            detail: format!("manifest is corrupt — refusing to collect: {e}"),
        })?;
        if pruned.contains(n) {
            continue;
        }
        for state in manifest.modules.values() {
            referenced.insert(utf8_path(&state.store_path)?);
            // Retained generations pin the consumer's transitive build closure.
            for path in &state.build_closure {
                referenced.insert(utf8_path(path)?);
            }
        }
        // 0015 §4: generations pin prior blobs the same way — a prior
        // is restorable exactly while its generation lives
        for state in manifest.modules.values() {
            for entry in &state.entries {
                if let Some(store::Prior::File { hash, .. }) = &entry.prior {
                    referenced.insert(utf8_path(&store::prior_blob_path(home, hash))?);
                }
            }
        }
    }
    if !dry_run {
        for n in &pruned {
            std::fs::remove_dir_all(store::generation_dir(home, *n))?;
        }
    }

    let referenced: Vec<&str> = referenced.iter().map(String::as_str).collect();

    // The deletion kernel (0046): delete == candidates minus roots,
    // extensionally — and monotone in the root set.
    let store_dir = home.join(store::STORE_DIR);
    if store_dir.is_dir() {
        let mut candidates: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&store_dir)? {
            candidates.push(utf8_path(&entry?.path())?);
        }
        let candidate_refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
        for doomed in gripsack_policy::retention::plan_delete(&referenced, &candidate_refs) {
            let path = Path::new(doomed);
            report.bytes_freed += dir_size(path)?;
            if !dry_run {
                std::fs::remove_dir_all(path)?;
            }
            report.store_removed.push(path.to_path_buf());
        }
    }
    // prior blobs (0015 §4): same reachability rule, flat dir of files
    let prior_dir = home.join("prior");
    if prior_dir.is_dir() {
        let mut candidates: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&prior_dir)? {
            candidates.push(utf8_path(&entry?.path())?);
        }
        let candidate_refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
        for doomed in gripsack_policy::retention::plan_delete(&referenced, &candidate_refs) {
            let path = Path::new(doomed);
            report.bytes_freed += dir_size(path)?;
            if !dry_run {
                std::fs::remove_file(path)?;
            }
            report.store_removed.push(path.to_path_buf());
        }
    }
    Ok(report)
}

/// GC inventory and roots are path strings (0046): a non-UTF-8 entry
/// cannot be compared for membership honestly, so admission refuses —
/// the same rule the journal adopted in 0045 F1. Fail closed: no
/// deletion plan over a partial inventory.
fn utf8_path(path: &Path) -> Result<String, ExecError> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| ExecError::Step {
            module: "*".into(),
            step: "gc".into(),
            detail: format!(
                "{} is not valid UTF-8 — refusing to compute a deletion set",
                path.display()
            ),
        })
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
            // the query canonicalizes like any declaration (0035 F1):
            // spelling never decides ownership
            let query = gripsack_store::canonical_dest(path)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| path.to_string());
            if entry.key() == query {
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
                build_only: false,
                intents: vec![],
                verified: None,
                entries: vec![store::DeployedEntry {
                    from: "a".into(),
                    to: "~/.config/m/a".into(),
                    key: None,
                    mode: Ownership::TrackedCopy,
                    vars: Default::default(),
                    hash: gripsack_store::hash::ManifestHash::from_raw("a".repeat(64)),
                    file_mode: None,
                    source_executable: None,
                    prior: None,
                    preserved_drift: false,
                }],
                env: vec![],
                tree256: None,
                build_closure: vec![],
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
        let report = gc(
            &crate::LifecycleSession::acquire(home).unwrap(),
            None,
            false,
        )
        .unwrap();
        assert_eq!(report.store_removed.len(), 1);
        assert!(report.store_removed[0].ends_with("zzz-orphan"));
        assert!(home.join("store/aaa-m").exists());
        assert!(report.bytes_freed > 0);
    }

    #[test]
    fn keep_generations_prunes_oldest_and_their_paths() {
        let dir = setup();
        let home = dir.path();
        let report = gc(
            &crate::LifecycleSession::acquire(home).unwrap(),
            Some(2),
            false,
        )
        .unwrap();
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
        let report = gc(
            &crate::LifecycleSession::acquire(home).unwrap(),
            Some(1),
            false,
        )
        .unwrap();
        assert!(store::generation_dir(home, 3).exists());
        assert!(!report.generations_removed.contains(&3));
        assert!(home.join("store/ccc-m").exists());
    }

    /// The F3 scenario (0045): a crash between record and flip leaves
    /// a journaled prior blob that NO retained manifest references —
    /// collecting it would make recovery impossible.
    fn setup_crash_window() -> (tempfile::TempDir, PathBuf) {
        let dir = setup();
        let home = dir.path();
        let cap = gripsack_fs::open_or_create(home).unwrap();
        // a run declared its target and journaled one destination
        store::journal::begin_run(&cap, Some(3), 4, store::journal::RunOp::Apply).unwrap();
        let dest = home.join("dest.txt");
        fs::write(&dest, b"user bytes\n").unwrap();
        let blob = store::journal::store_prior_blob_in(&cap, b"user bytes\n").unwrap();
        let prior = store::journal::Prior::File {
            hash: blob.clone(),
            mode: 0o644,
        };
        store::journal::record(&cap, &dest, &prior, &store::journal::Intended::Removed).unwrap();
        // sanity: the blob exists and no manifest references it
        let blob_path = home.join("prior").join(&blob);
        assert!(blob_path.exists());
        (dir, blob_path)
    }

    #[test]
    fn unfinished_recovery_blocks_collection() {
        let (dir, blob) = setup_crash_window();
        let home = dir.path();
        let session = crate::LifecycleSession::acquire(home).unwrap();
        for dry_run in [false, true] {
            let err = gc(&session, Some(1), dry_run)
                .expect_err("gc must refuse while recovery is pending");
            let text = err.to_string();
            assert!(text.contains("recovery state is pending"), "{text}");
            assert!(text.contains("reconcile"), "{text}");
        }
        // nothing was deleted — not the blob, not the orphan, no generation
        assert!(blob.exists());
        assert!(home.join("store/zzz-orphan").exists());
        assert_eq!(store::list_generations(home).unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn reconciled_journal_unblocks_collection() {
        let (dir, blob) = setup_crash_window();
        let home = dir.path();
        // the crash is reconciled (as the next apply would): the
        // destination did not exist before the run, so the
        // journaled state is consumed and the journal drains
        let notes =
            store::journal::reconcile(&gripsack_fs::open_or_create(home).unwrap(), home).unwrap();
        assert!(!notes.is_empty());
        let session = crate::LifecycleSession::acquire(home).unwrap();
        gc(&session, Some(1), false).unwrap();
        // the blob's generation pins lapsed WITH the journal — safe
        assert!(!blob.exists());
    }

    #[test]
    fn quarantined_entries_block_collection() {
        let (dir, _blob) = setup_crash_window();
        let home = dir.path();
        // reconcile refuses the (fabricated, tag-valid but
        // blob-missing) entry? No: fabricate a MALFORMED entry so
        // reconcile quarantines it, then gc must still refuse.
        let cap = gripsack_fs::open_or_create(home).unwrap();
        fs::write(home.join("journal").join("garbage.json"), b"not json").unwrap();
        assert!(store::journal::reconcile(&cap, home).is_err());
        let session = crate::LifecycleSession::acquire(home).unwrap();
        let err = gc(&session, Some(1), true).expect_err("quarantine blocks gc");
        assert!(err.to_string().contains("quarantined"), "{err}");
        assert!(home.join("journal/quarantine/garbage.json").exists());
    }

    #[test]
    fn a_session_for_another_home_authorizes_nothing_here() {
        // F4: the session carries its home — gc cannot be pointed at
        // a different home than the lock covers.
        let (dir, _blob) = setup_crash_window();
        let other = tempfile::tempdir().unwrap();
        let session = crate::LifecycleSession::acquire(other.path()).unwrap();
        assert_ne!(session.home(), dir.path());
        // using the session operates on ITS home (empty — nothing to
        // collect), never on `dir`'s: the API takes no home argument.
        let report = gc(&session, None, true).unwrap();
        assert!(report.store_removed.is_empty());
        assert_eq!(store::list_generations(dir.path()).unwrap(), vec![1, 2, 3]);
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
