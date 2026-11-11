//! `gc` and `why-owns` — the store's hygiene commands (0001 §gc).
//!
//! Generations pin store paths; anything no retained generation references
//! is collectable. `keep_generations` bounds how many generations live
//! (user config `~/.config/gripsack/config.toml`); the current generation is
//! never touched.
//!
//! The kernel's GC machine (`gripsack_kernel::gc`, plan/0056 F2 step 4)
//! decides every deletion; [`gc`] is its driver. It answers each read with
//! admitted facts (inventory, manifests, roots, candidates) and executes only
//! the effects it is commanded, re-checking each command's `expect` first.

mod directory_roots;
mod inventory;
#[cfg(test)]
mod root_model;

use crate::ctx::ExecError;
use gripsack_host::store;
use gripsack_kernel::gc::{
    Command, Event, GcState, InventoryFacts, Mode, Refusal as GcRefusal, gc_step,
};
use gripsack_kernel::recover::Failure;
use gripsack_kernel::retention::{ManifestFacts, Scope};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct GcReport {
    pub generations_removed: Vec<store::GenerationId>,
    pub store_removed: Vec<PathBuf>,
    /// Stale `generations/.staging-*` left by a publisher that died before
    /// its rename (plan/0058 item 4).
    pub staging_removed: Vec<PathBuf>,
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
/// convention. The session is also what makes every staging directory
/// stale: publishers hold it.
pub fn gc(
    session: &crate::LifecycleSession,
    keep: Option<u32>,
    dry_run: bool,
) -> Result<GcReport, ExecError> {
    let mut host = Host::open(session.home(), dry_run)?;
    let mode = if dry_run {
        Mode::Preview
    } else {
        Mode::Collect
    };
    let keep = keep.map(|keep| keep as usize);
    let mut state = GcState::new(mode, keep);
    let mut event = Event::Start;
    loop {
        let (next, command) = gc_step(state, event);
        state = next;
        event = match command {
            Command::ReadInventory => Event::Inventory(host.inventory()?),
            Command::ReadManifest(number) => Event::Manifest(host.manifest(number)?),
            Command::ReadRoots => Event::Roots(host.roots()?),
            Command::ReadCandidates => Event::Candidates(host.candidates()?),
            Command::Plan => {
                host.plan(&state)?;
                Event::Done
            }
            Command::SealFloor => {
                host.generations
                    .admit_retirements(host.report.generations_removed.len())?;
                host.generations.seal_floor()?;
                Event::Done
            }
            Command::Retire {
                generation,
                current,
            } => {
                // re-check: `current` is still what the inventory said
                if store::generations::current_in(&host.home_path, &host.home)? != current {
                    Event::Failed(Failure::Drift)
                } else {
                    host.generations.retire(generation)?;
                    Event::Done
                }
            }
            Command::Barrier => {
                host.generations.seal()?;
                Event::Done
            }
            Command::Delegate => {
                host.retire_receipts()?;
                Event::Done
            }
            Command::Remove { candidate, expect } => {
                if host.collectable()?.remove(candidate, expect)? {
                    Event::Done
                } else {
                    Event::Failed(Failure::Drift)
                }
            }
            Command::Finish(_) => return Ok(host.report),
            Command::Abort(refusal) => return Err(host.refusal(refusal)),
        };
    }
}

/// What the driver holds between commands: the pinned home, the admitted
/// generation directory, the decoded manifests (the directory domain's
/// retirement needs them), and the collectable entries.
struct Host {
    home_path: PathBuf,
    home: gripsack_host::fs::Dir,
    dry_run: bool,
    generations: store::generations::GenerationDirectory,
    journal_pending: bool,
    activation_pending: bool,
    current: Option<store::GenerationId>,
    manifests: BTreeMap<store::GenerationId, store::Generation>,
    retirements: BTreeMap<store::GenerationId, store::Generation>,
    collectable: Option<inventory::Collectable>,
    report: GcReport,
}

impl Host {
    fn open(home_path: &Path, dry_run: bool) -> Result<Self, ExecError> {
        // fail closed (0027 §2): enumeration errors propagate — the
        // deletion set derives from this inventory, and "cannot read" must
        // never read as "nothing referenced". An unreadable journal fails
        // the same way (pending_recovery errors).
        let home = if dry_run {
            gripsack_host::fs::open(home_path)?
        } else {
            gripsack_host::fs::open_or_create(home_path)?
        };
        let journal_pending = store::journal::pending_recovery(&home)?.is_some();
        let activation_pending = store::activation::has_pending(&home)?;
        let generations = store::generations::GenerationDirectory::open(&home)?;
        let current = store::generations::current_in(home_path, &home)?;
        Ok(Self {
            home_path: home_path.to_path_buf(),
            home,
            dry_run,
            generations,
            journal_pending,
            activation_pending,
            current,
            manifests: BTreeMap::new(),
            retirements: BTreeMap::new(),
            collectable: None,
            report: GcReport::default(),
        })
    }

    fn inventory(&self) -> Result<InventoryFacts, ExecError> {
        Ok(InventoryFacts {
            recovery_pending: self.journal_pending || self.activation_pending,
            current: self.current,
            generations: self.generations.inventory().clone(),
        })
    }

    /// Admit one manifest through the pinned inventory and hand the kernel
    /// what it pins: its receipts' predecessor generations, its deployed
    /// store paths, their build closures, and its entries' prior blobs
    /// (0015 §4: a prior is restorable exactly while its generation lives).
    fn manifest(&mut self, number: store::GenerationId) -> Result<ManifestFacts, ExecError> {
        let manifest = if self.dry_run {
            self.generations.inspect_manifest(&self.home_path, number)?
        } else {
            self.generations
                .admit_manifest(&self.home, &self.home_path, number)?
        };
        let predecessors = manifest
            .directory_transitions
            .iter()
            .map(|receipt| receipt.predecessor_generation)
            .chain(manifest.migration.iter().map(|receipt| receipt.predecessor))
            .collect();
        let mut roots = Vec::new();
        for state in manifest.deployments.values() {
            roots.push(utf8_path(&state.store_path)?.into_bytes());
            for path in &state.build_closure {
                roots.push(utf8_path(path)?.into_bytes());
            }
            for entry in &state.entries {
                if let Some(store::Prior::File { hash, .. }) = &entry.prior {
                    roots.push(utf8_path(&hash.path_in(&self.home_path))?.into_bytes());
                }
            }
        }
        self.manifests.insert(number, manifest);
        Ok(ManifestFacts {
            id: number,
            predecessors,
            roots,
        })
    }

    /// Roots outside the generations (ring 3, tested, not proved): workspace
    /// consumer roots and retained schedule roots. Pending directory
    /// retirements are admitted here too, before any effect.
    fn roots(&mut self) -> Result<Vec<Vec<u8>>, ExecError> {
        self.retirements =
            directory_roots::pending(&self.generations, &self.home, &self.home_path, self.dry_run)?;
        let mut roots: BTreeSet<String> =
            crate::workspace::roots::inventory(&self.home, &self.home_path)?;
        for path in store::activation::retained_schedule_roots(&self.home, &self.home_path)? {
            roots.insert(utf8_path(&path)?);
        }
        Ok(roots.into_iter().map(String::into_bytes).collect())
    }

    fn candidates(&mut self) -> Result<Vec<gripsack_kernel::retention::Candidate>, ExecError> {
        let collectable = inventory::Collectable::open(&self.home, &self.home_path)?;
        let candidates = collectable.candidates()?;
        self.collectable = Some(collectable);
        Ok(candidates)
    }

    fn collectable(&self) -> io::Result<&inventory::Collectable> {
        self.collectable
            .as_ref()
            .ok_or_else(|| io::Error::other("GC removal before inventory"))
    }

    /// Report the machine's decision and size it: every fallible size read
    /// completes before the first effect.
    fn plan(&mut self, state: &GcState) -> Result<(), ExecError> {
        self.report.generations_removed = state.retired().collect();
        let collectable = self.collectable()?;
        let mut store_removed = Vec::new();
        let mut staging_removed = Vec::new();
        for &candidate in &state.doomed {
            let path = collectable.path(candidate).to_path_buf();
            match collectable.scope(candidate) {
                Scope::Store | Scope::Prior => store_removed.push(path),
                Scope::Staging => staging_removed.push(path),
            }
        }
        let bytes = inventory::total(state.doomed.iter().map(|&c| collectable.size(c)))?;
        self.report.store_removed = store_removed;
        self.report.staging_removed = staging_removed;
        self.report.bytes_freed = bytes;
        Ok(())
    }

    /// The directory domain (plan/0056 F2 step 9): retire the receipts the
    /// retired generations and earlier interrupted retirements hold, then
    /// clear finished retirements.
    fn retire_receipts(&self) -> Result<(), ExecError> {
        let retired: BTreeSet<_> = self.report.generations_removed.iter().copied().collect();
        directory_roots::retire(
            &self.generations,
            &self.manifests,
            &retired,
            &self.retirements,
        )?;
        self.generations.cleanup_retired()?;
        Ok(())
    }

    fn refusal(&self, refusal: GcRefusal) -> ExecError {
        let detail = match refusal {
            // recovery state is load-bearing: a journaled prior blob may be
            // referenced by NO retained manifest, so collecting before
            // reconcile would destroy the bytes recovery needs. Refused
            // dry-run included: the deletion set is unsound while recovery
            // is pending, and a preview that lies is worse than no preview.
            GcRefusal::RecoveryPending => format!(
                "recovery state is pending (journal={}, activation={}) — run `grip apply` or \
                 `grip rollback` to reconcile before collecting; nothing was deleted",
                self.journal_pending, self.activation_pending
            ),
            GcRefusal::CorruptCurrent => format!(
                "current generation {} has no directory on disk — refusing to collect",
                self.current
                    .map_or_else(|| "?".to_string(), |current| current.to_string())
            ),
            GcRefusal::MissingPredecessor { generation } => format!(
                "generation {generation}: directory receipt predecessor generation is missing; \
                 refusing GC"
            ),
            GcRefusal::Drifted => {
                "an object changed between inventory and removal — run gc again".to_string()
            }
            GcRefusal::Malformed | GcRefusal::Failed | GcRefusal::Protocol => {
                format!("collection stopped: {refusal:?}")
            }
        };
        ExecError::Step {
            output: "*".into(),
            step: "gc".into(),
            detail,
        }
    }
}

/// GC inventory and roots are path strings (0046): a non-UTF-8 entry
/// cannot be compared for membership honestly, so admission refuses —
/// the same rule the journal adopted in 0045 F1. Fail closed: no
/// deletion plan over a partial inventory.
fn utf8_path(path: &Path) -> Result<String, ExecError> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| ExecError::Step {
            output: "*".into(),
            step: "gc".into(),
            detail: format!(
                "{} is not valid UTF-8 — refusing to compute a deletion set",
                path.display()
            ),
        })
}

/// All ownership units of a destination belonging to one deployment or profile.
#[derive(Debug)]
pub struct PathOwner {
    pub name: String,
    pub entries: Vec<store::DeployedEntry>,
}

/// Owners of a deployed path in the current generation. A managed-block
/// destination may have several owners; never hide all but the first.
pub fn why_owns(home: &Path, path: &str) -> Result<Vec<PathOwner>, ExecError> {
    let Some(n) = store::current_generation(home)? else {
        return Ok(Vec::new());
    };
    let query = gripsack_host::store::canonical_dest(path)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_owned());
    let manifest = store::read_manifest(home, n)?;
    let mut owners = Vec::new();
    for (name, state) in manifest.deployments {
        let entries: Vec<_> = state
            .entries
            .into_iter()
            .filter(|entry| entry.key() == query)
            .collect();
        if !entries.is_empty() {
            owners.push(PathOwner { name, entries });
        }
    }
    Ok(owners)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_host::store::Ownership;
    use std::fs;
    use store::GenerationId;

    fn mk_gen(n: u64, store_path: PathBuf) -> store::Generation {
        let mut deployments = std::collections::BTreeMap::new();
        deployments.insert(
            "m".to_string(),
            store::DeploymentState {
                store_path,
                build_only: false,
                intents: vec![],
                verified: None,
                entries: vec![store::DeployedEntry {
                    from: "a".into(),
                    to: "~/.config/m/a".into(),
                    key: None,
                    ownership: store::StoredOwnership::Legacy(Ownership::TrackedCopy),
                    vars: Default::default(),
                    hash: gripsack_host::store::hash::ManifestHash::from_raw("a".repeat(64)),
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
        store::Generation {
            number: GenerationId::new(n),
            deployments,
            directory_transitions: Vec::new(),
            migration: None,
        }
    }

    fn setup() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        for (n, tag) in [(1, "aaa"), (2, "bbb"), (3, "ccc")] {
            let sp = home.join("store").join(format!("{tag}-m"));
            fs::create_dir_all(&sp).unwrap();
            fs::write(sp.join("payload"), format!("gen {n}")).unwrap();
            store::write_manifest(
                &gripsack_host::fs::open_or_create(home).unwrap(),
                &mk_gen(n, sp),
            )
            .unwrap();
        }
        // an orphan path no manifest references
        let orphan = home.join("store").join("zzz-orphan");
        fs::create_dir_all(&orphan).unwrap();
        fs::write(orphan.join("payload"), b"old").unwrap();
        std::os::unix::fs::symlink("generations/3", home.join("current")).unwrap();
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
        assert_eq!(report.store_removed, vec![home.join("store/zzz-orphan")]);
        assert!(home.join("store/aaa-m").exists());
        assert_eq!(report.bytes_freed, 3);
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
        assert_eq!(report.generations_removed, vec![GenerationId::new(1)]);
        assert!(!store::generation_dir(home, GenerationId::new(1)).exists());
        assert!(store::generation_dir(home, GenerationId::new(3)).exists());
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

    /// plan/0058 item 4: a publisher that died between staging and rename
    /// leaves `generations/.staging-<N>-<pid>`. Under the lifecycle session
    /// no publisher is live, so every canonical staging directory is stale.
    #[test]
    fn stale_generation_staging_is_reaped() {
        let dir = setup();
        let home = dir.path();
        let stale = home.join("generations/.staging-4-4194304");
        fs::create_dir_all(stale.join("env")).unwrap();
        fs::write(stale.join("manifest.json"), b"{}").unwrap();
        fs::write(stale.join("env/profile.sh"), b"export A=1\n").unwrap();
        // not canonical: never ours to remove
        let foreign = home.join("generations/.staging-notes");
        fs::create_dir_all(&foreign).unwrap();
        let session = crate::LifecycleSession::acquire(home).unwrap();
        gc(&session, None, true).unwrap();
        assert!(stale.exists(), "a dry run deletes nothing");
        gc(&session, None, false).unwrap();
        assert!(!stale.exists(), "stale staging must be reaped");
        assert!(foreign.exists());
        assert_eq!(
            &*store::list_generations(home).unwrap(),
            &[
                GenerationId::new(1),
                GenerationId::new(2),
                GenerationId::new(3)
            ]
        );
    }

    /// The F3 scenario (0045): a crash between record and flip leaves
    /// a journaled prior blob that NO retained manifest references —
    /// collecting it would make recovery impossible.
    fn setup_crash_window() -> (tempfile::TempDir, PathBuf) {
        let dir = setup();
        let home = dir.path();
        let cap = gripsack_host::fs::open_or_create(home).unwrap();
        // a run declared its target and journaled one destination
        let run = store::journal::begin_run(
            &cap,
            home,
            Some(GenerationId::new(3)),
            GenerationId::new(4),
            store::journal::RunOp::Apply,
        )
        .unwrap();
        let dest = home.join("dest.txt");
        fs::write(&dest, b"user bytes\n").unwrap();
        let directory = gripsack_host::fs::open(home).unwrap();
        let observed = store::journal::live_identity(&directory, Path::new("dest.txt")).unwrap();
        // the process dies after the entry is durable, before the removal
        let crash = |_: &[u8], _: Option<gripsack_host::storage::Permissions>| {
            Err(std::io::Error::other("simulated crash"))
        };
        store::journal::mutate(
            &run,
            directory,
            PathBuf::from("dest.txt"),
            &dest,
            observed.as_ref(),
            &store::journal::Intended::Removed,
            store::journal::Install::Splice(Box::new(crash)),
        )
        .expect_err("the simulated crash");
        let blobs: Vec<PathBuf> = fs::read_dir(home.join("prior"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        let [blob_path] = blobs.as_slice() else {
            panic!("the actual destination must produce one retained file prior: {blobs:?}");
        };
        let blob_path = blob_path.clone();
        assert!(blob_path.exists());
        (dir, blob_path)
    }

    #[test]
    fn unfinished_recovery_blocks_collection() {
        let (dir, blob) = setup_crash_window();
        let home = dir.path();
        let session = crate::LifecycleSession::acquire(home).unwrap();
        for dry_run in [false, true] {
            gc(&session, Some(1), dry_run).expect_err("gc must refuse while recovery is pending");
        }
        // nothing was deleted — not the blob, not the orphan, no generation
        assert!(blob.exists());
        assert!(home.join("store/zzz-orphan").exists());
        assert_eq!(
            &*store::list_generations(home).unwrap(),
            &[
                GenerationId::new(1),
                GenerationId::new(2),
                GenerationId::new(3)
            ]
        );
    }

    #[test]
    fn reconciled_journal_unblocks_collection() {
        let (dir, blob) = setup_crash_window();
        let home = dir.path();
        // the crash is reconciled (as the next apply would): the
        // destination did not exist before the run, so the
        // journaled state is consumed and the journal drains
        store::journal::reconcile(&gripsack_host::fs::open_or_create(home).unwrap(), home).unwrap();
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
        let cap = gripsack_host::fs::open_or_create(home).unwrap();
        fs::write(home.join("journal").join("garbage.json"), b"not json").unwrap();
        assert!(store::journal::reconcile(&cap, home).is_err());
        let session = crate::LifecycleSession::acquire(home).unwrap();
        gc(&session, Some(1), true).expect_err("quarantine blocks gc");
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
        assert_eq!(
            &*store::list_generations(dir.path()).unwrap(),
            &[
                GenerationId::new(1),
                GenerationId::new(2),
                GenerationId::new(3)
            ]
        );
    }

    #[test]
    fn why_owns_finds_the_owner() {
        let dir = setup();
        let home = dir.path();
        let owners = why_owns(home, "~/.config/m/a").unwrap();
        assert_eq!(owners[0].name, "m");
        assert_eq!(
            owners[0].entries[0].ownership.policy(),
            Ownership::TrackedCopy
        );
        let absolute = gripsack_host::store::expand_home("~/.config/m/a");
        assert_eq!(
            why_owns(home, &absolute.to_string_lossy()).unwrap()[0].name,
            "m"
        );
        assert!(why_owns(home, "/nope").unwrap().is_empty());
    }
}
