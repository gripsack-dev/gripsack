//! Deploy: ownership modes, drift, destinations (0001 §3.7).

use crate::ctx::{Ctx, ExecError};
use crate::report::ReportKind;
use gripsack_ir::{Entry, Ownership};
use gripsack_store as store;
use gripsack_store::expand_home;
use std::path::Path;

/// Restore one destination to what a generation's manifest recorded —
/// the ONE deploy-restore path, shared by rollback (0001 §3.5): every
/// mode gets its correct semantics, never a naive byte copy (template
/// re-renders with the recorded vars; merge re-upserts only the block
/// into the foreign file).
pub fn restore_entry(
    dest: &Path,
    entry: &store::DeployedEntry,
    store_path: &Path,
    module: &str,
) -> std::io::Result<()> {
    let Some(plan) = compute_restore(dest, entry, store_path, module)? else {
        tracing::warn!(?dest, "restore skipped — leaving destination as-is");
        return Ok(());
    };
    let (dest_dir, dest_name) = dest_capability(dest)?;
    execute_restore(&dest_dir, &dest_name, &plan)
}

/// What a restore intends to land and the journal identity of that
/// end state — computed BEFORE any mutation (0026 §6), so the journal
/// records intent, never observation-after-the-fact.
pub struct RestorePlan {
    /// Journal identity after the restore: link target for owned,
    /// canonical bytes hash otherwise.
    pub intent: String,
    pub write: RestoreWrite,
}

pub enum RestoreWrite {
    /// Owned: point the destination link here.
    Link(std::path::PathBuf),
    /// Tracked copy / rendered template / merge-upserted whole file.
    Bytes(Vec<u8>),
}

/// Plan the restore of one deployed entry without touching the
/// destination. None = leave it alone (an unreadable foreign merge
/// file, or an owned link whose store source is gone — the manifest
/// is stale, and a dangling link is worse than an absent dest).
pub fn compute_restore(
    dest: &Path,
    entry: &store::DeployedEntry,
    store_path: &Path,
    module: &str,
) -> std::io::Result<Option<RestorePlan>> {
    let source = store_path.join(&entry.from);
    match entry.mode {
        Ownership::Owned => {
            if !source.exists() {
                return Ok(None);
            }
            Ok(Some(RestorePlan {
                intent: source.to_string_lossy().into_owned(),
                write: RestoreWrite::Link(source),
            }))
        }
        Ownership::Merge => {
            let payload = std::fs::read_to_string(&source).unwrap_or_default();
            // a dest that is not text cannot host a managed block:
            // splicing onto "" would REPLACE the whole foreign file
            // (silent data loss) — leave it alone instead
            let Some(existing) = read_foreign_text(dest) else {
                return Ok(None);
            };
            match crate::template::upsert_block(&existing, module, dest, None, &payload) {
                Ok(new) => Ok(Some(RestorePlan {
                    intent: store::canonical_bytes_hash(new.as_bytes()),
                    write: RestoreWrite::Bytes(new.into_bytes()),
                })),
                Err(_) => Ok(None), // malformed markers: leave the foreign file alone
            }
        }
        Ownership::Template => {
            let rendered = crate::template::render_template(
                &std::fs::read(&source)?,
                &entry.vars,
                &entry.from,
            )
            .map_err(std::io::Error::other)?;
            Ok(Some(RestorePlan {
                intent: store::canonical_bytes_hash(&rendered),
                write: RestoreWrite::Bytes(rendered),
            }))
        }
        Ownership::TrackedCopy => {
            let bytes = std::fs::read(&source)?;
            Ok(Some(RestorePlan {
                intent: store::canonical_bytes_hash(&bytes),
                write: RestoreWrite::Bytes(bytes),
            }))
        }
    }
}

/// Land a planned restore through the pinned destination capability.
pub fn execute_restore(
    dest_dir: &gripsack_fs::Dir,
    dest_name: &Path,
    plan: &RestorePlan,
) -> std::io::Result<()> {
    match &plan.write {
        RestoreWrite::Link(target) => gripsack_fs::symlink_replace(dest_dir, dest_name, target),
        RestoreWrite::Bytes(bytes) => gripsack_fs::atomic_write(dest_dir, dest_name, bytes),
    }
}

/// Remove a destination we deployed, with drift guards (0001 §3.5):
/// never delete user edits. `Ok(false)` is the drift POLICY (kept);
/// every I/O failure is Err — a failed removal must never read as
/// "user drift, kept" (0027 §1). Everything goes through the pinned
/// parent capability the caller opened (0027 §5). Merge entries
/// remove only our block from the foreign file.
pub fn remove_entry_deployed(
    dest_dir: &gripsack_fs::Dir,
    dest_name: &Path,
    entry: &store::DeployedEntry,
    module: &str,
) -> std::io::Result<bool> {
    match entry.mode {
        Ownership::Owned => {
            let ours = dest_dir
                .read_link_contents(dest_name)
                .map(|t| t.starts_with(store::gripsack_home()))
                .unwrap_or(false);
            if !ours {
                return Ok(false);
            }
            remove_if_present(dest_dir, dest_name)?;
            Ok(true)
        }
        Ownership::Merge => {
            let existing = match dest_dir.read_to_string(dest_name) {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
                Err(e) => return Err(e),
            };
            match crate::template::extract_block(&existing, module) {
                Some(content) if store::canonical_bytes_hash(content.as_bytes()) == entry.hash => {
                    let new = crate::template::remove_block(&existing, module)
                        .expect("block found above");
                    if new.trim().is_empty() {
                        remove_if_present(dest_dir, dest_name)?;
                    } else {
                        gripsack_fs::atomic_write(dest_dir, dest_name, new.as_bytes())?;
                    }
                    Ok(true)
                }
                _ => Ok(false), // drifted block is the user's now
            }
        }
        _ => {
            // copy-like: only delete bytes identical to what we wrote
            let current = match store::canonical_file_hash_in(dest_dir, dest_name) {
                Ok(h) => h,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
                Err(e) => return Err(e),
            };
            if current != entry.hash {
                return Ok(false);
            }
            remove_if_present(dest_dir, dest_name)?;
            Ok(true)
        }
    }
}

/// remove_file where NotFound is success (the goal state), anything
/// else is a real error (0027 §1).
fn remove_if_present(dest_dir: &gripsack_fs::Dir, dest_name: &Path) -> std::io::Result<()> {
    match dest_dir.remove_file(dest_name) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Record what a destination is before a take-over absorbs it (0015
/// §4): real-file bytes go to the content-addressed prior blob store,
/// a symlink's target is recorded verbatim. None = nothing there (or
/// unreadable) — default removal semantics then apply.
/// Strictly fallible (0025 §E): only NotFound means "no prior".
/// Every other read, metadata, encoding, or blob-storage failure
/// aborts the take-over BEFORE the mutation — recording `prior: None`
/// for a file that existed but could not be captured would break the
/// central promise (exact pre-adoption restoration).
fn capture_prior(
    dest_dir: &gripsack_fs::Dir,
    dest_name: &Path,
    home: &gripsack_fs::Dir,
) -> std::io::Result<Option<store::Prior>> {
    let meta = match dest_dir.symlink_metadata(dest_name) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if meta.file_type().is_symlink() {
        let target = dest_dir.read_link_contents(dest_name)?;
        let Some(target) = target.to_str() else {
            // same refusal as journal::capture: a lossily recorded
            // target restores as a DIFFERENT link
            return Err(std::io::Error::other(format!(
                "symlink target is not UTF-8 ({} bytes) — cannot record the prior state",
                target.as_os_str().len()
            )));
        };
        Ok(Some(store::Prior {
            kind: store::PriorKind::Symlink,
            content: Some(target.to_string()),
            mode: None,
        }))
    } else if meta.is_file() {
        let bytes = dest_dir.read(dest_name)?;
        let sha = store::journal::store_prior_blob_in(home, &bytes)?;
        #[cfg(unix)]
        let mode = {
            use gripsack_fs::cap_std::fs::MetadataExt;
            Some(meta.mode() & 0o777)
        };
        #[cfg(not(unix))]
        let mode = None;
        Ok(Some(store::Prior {
            kind: store::PriorKind::File,
            content: Some(sha),
            mode,
        }))
    } else {
        Ok(None)
    }
}

/// Write a prior state back to its destination (0015 §4). Every
/// failure is an error (0027 §1): a prior that cannot be read,
/// written, or chmod'd must abort the transaction — the bool era read
/// those as "kept", committing a generation over a failed restore.
/// The recorded mode rides the write exactly (0027 §6).
fn restore_prior(
    dest_dir: &gripsack_fs::Dir,
    dest_name: &Path,
    prior: &store::Prior,
    home: &Path,
) -> std::io::Result<()> {
    match prior.kind {
        store::PriorKind::File => {
            let sha = prior.content.as_deref().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "a file prior without a blob hash (corrupt manifest)",
                )
            })?;
            let bytes = std::fs::read(store::prior_blob_path(home, sha))?;
            #[cfg(unix)]
            match prior.mode {
                Some(mode) => {
                    gripsack_fs::atomic_write_with_mode(dest_dir, dest_name, &bytes, mode)?
                }
                None => gripsack_fs::atomic_write(dest_dir, dest_name, &bytes)?,
            }
            #[cfg(not(unix))]
            gripsack_fs::atomic_write(dest_dir, dest_name, &bytes)?;
            Ok(())
        }
        store::PriorKind::Symlink => {
            let target = prior.content.as_deref().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "a symlink prior without a target (corrupt manifest)",
                )
            })?;
            // symlink_replace over remove+create: the swap is atomic
            // and parent-fsync'd (strictly stronger than the old pair)
            gripsack_fs::symlink_replace(dest_dir, dest_name, Path::new(target))
        }
    }
}

/// Rollback/prune for a deployed entry (0015 §4): when the destination
/// is still exactly what gripsack deployed and a prior exists, restore
/// the original file/symlink — "your original files have been
/// restored." Drifted destinations and prior-less entries fall back to
/// the drift-guarded removal.
/// [`intact_deployed`] through the pinned capability (0027 §5).
pub fn intact_deployed_relative(
    dest_dir: &gripsack_fs::Dir,
    dest_name: &Path,
    entry: &store::DeployedEntry,
    store_path: &Path,
) -> bool {
    match entry.mode {
        // intact means EXACTLY this entry's store link (0029 §11):
        // "points somewhere under gripsack" conflated a user-repointed
        // link with ownership
        Ownership::Owned => dest_dir
            .read_link_contents(dest_name)
            .map(|t| t == store_path.join(&entry.from))
            .unwrap_or(false),
        Ownership::Merge => false, // merge never carries a prior
        _ => store::canonical_file_hash_in(dest_dir, dest_name)
            .map(|h| h == entry.hash)
            .unwrap_or(false),
    }
}

/// Is the destination still exactly what this manifest entry
/// deployed? (Merge blocks are checked by block hash at the call
/// sites — a foreign file is never "intact" as a whole.)
pub fn intact_deployed(dest: &Path, entry: &store::DeployedEntry, store_path: &Path) -> bool {
    match entry.mode {
        Ownership::Owned => std::fs::read_link(dest)
            .map(|t| t == store_path.join(&entry.from))
            .unwrap_or(false),
        Ownership::Merge => false, // merge never carries a prior
        _ => store::canonical_file_hash(dest)
            .map(|h| h == entry.hash)
            .unwrap_or(false),
    }
}

/// The intended post-prune identity of a destination (0026 §6):
/// the restored prior's identity when a prior exists, REMOVED
/// otherwise. Known BEFORE the mutation, from the prior blob —
/// never observed afterward.
pub fn prune_intent(entry: &store::DeployedEntry, home: &Path) -> std::io::Result<String> {
    match &entry.prior {
        Some(prior) => match prior.kind {
            store::PriorKind::File => {
                let sha = prior
                    .content
                    .as_deref()
                    .expect("a file prior carries its blob hash");
                let bytes = std::fs::read(store::prior_blob_path(home, sha))?;
                Ok(store::canonical_bytes_hash(&bytes))
            }
            store::PriorKind::Symlink => Ok(prior
                .content
                .clone()
                .expect("a symlink prior carries its target")),
        },
        None => Ok(store::journal::REMOVED.to_string()),
    }
}

/// Restore the recorded prior, or drift-guarded removal when there
/// is none (0015 §4). Callers prove intactness at plan time (apply's
/// prune and the rollback planner both check before journaling) — the
/// redundant re-check used to receive `home` where `store_path` was
/// expected and silently miscompared, deleting links it should have
/// restored (0029).
pub fn remove_or_restore_prior(
    dest_dir: &gripsack_fs::Dir,
    dest_name: &Path,
    entry: &store::DeployedEntry,
    module: &str,
    home: &Path,
) -> std::io::Result<bool> {
    if let Some(prior) = &entry.prior {
        restore_prior(dest_dir, dest_name, prior, home)?;
        return Ok(true);
    }
    remove_entry_deployed(dest_dir, dest_name, entry, module)
}

/// One destination mutation, journaled for crash recovery (0019):
/// the prior state is recorded (file bytes into the prior blob
/// store) BEFORE the write and the post-mutation identity noted
/// after — a kill anywhere in between leaves an uncommitted entry
/// the next run's reconcile restores. The entry clears when the run
/// commits (the flip); per-entry there is no commit, matching the
/// run-level rollback's all-or-nothing semantics.
/// The copy/template disposition decision as a pure function
/// (0029 §2 — the lineage model in the harness drives THIS code).
/// `prev` is (the previous manifest's hash, whether it was preserved
/// drift). The authorization rule that was missing: only
/// last-written managed content may be updated; preserved drift
/// NEVER promotes to authority — reconvergence (live == desired) is
/// the only way back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyPlan {
    /// Nothing live: create.
    Fresh,
    /// Live IS the desired content.
    Satisfied,
    /// Live is what gripsack last wrote (managed): authorized update.
    Update,
    /// Live is foreign or drifted: preserve and report.
    Preserve,
    /// Explicit user consent to absorb whatever is live.
    TakeOver,
}

pub(crate) fn plan_copy(
    desired: &str,
    live: Option<&str>,
    prev: Option<(&str, bool)>,
    take_over: bool,
) -> CopyPlan {
    let Some(live) = live else {
        return CopyPlan::Fresh;
    };
    // explicit consent/absorb ALWAYS captures the origin — even when
    // the bytes already match (adopt relies on this to open the epoch)
    if take_over {
        return CopyPlan::TakeOver;
    }
    if live == desired {
        return CopyPlan::Satisfied;
    }
    match prev {
        // managed and live is our last write: the clean update path
        // (never a fresh take-over — the epoch's origin stands)
        Some((written, false)) if live == written => CopyPlan::Update,
        // explicit consent outranks preservation: --take-over absorbs
        // whatever is live and begins a new epoch with it as origin
        _ if take_over => CopyPlan::TakeOver,
        // preserved drift never authorizes — only reconvergence
        // (handled above) ends the drift state
        _ => CopyPlan::Preserve,
    }
}

/// The transition precondition (0029 §3): what the live object must
/// be when the mutation lands.
#[derive(Debug, Clone)]
pub(crate) enum Expect {
    /// The destination must be ABSENT — anything appearing between
    /// the decision and the mutation aborts the run.
    Absent,
    /// The destination's live identity must equal this.
    Is(String),
}

pub(crate) fn journaled(
    home: &gripsack_fs::Dir,
    dest_dir: &gripsack_fs::Dir,
    dest_name: &Path,
    dest: &Path,
    intended: String,
    expected_before: Expect,
    mutate: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    // the live object must still be the one the drift decision was
    // made against — a write between decision and capture aborts
    // instead of clobbering it. (There is no portable content-CAS:
    // renameat2 RENAME_EXCHANGE is Linux-only. Capture and mutation
    // are back-to-back; the residual window is documented on the
    // safety page.)
    let live = gripsack_store::journal::live_identity(dest_dir, dest_name)?;
    let ok = match &expected_before {
        Expect::Absent => live.is_none(),
        Expect::Is(expected) => live.as_deref() == Some(expected.as_str()),
    };
    if !ok {
        return Err(std::io::Error::other(format!(
            "{} changed between the drift decision and the mutation — aborting; re-run to retry",
            dest.display()
        )));
    }
    // prior AND intended post-state are durable BEFORE the mutation
    // (0026 §6): reconcile's three-way decision never confuses a
    // post-crash user edit with the mutation
    let prior = gripsack_store::journal::capture(dest_dir, dest_name, dest, home)?;
    gripsack_store::journal::record(home, dest, &prior, &intended)?;
    mutate()?;
    // the transaction postcondition (0027 §1): a helper that returns
    // Ok without producing the intended state fails the run HERE, and
    // compensation restores the prior — the flip never commits an
    // unverified destination
    let live = gripsack_store::journal::live_identity(dest_dir, dest_name)?;
    let landed = if intended == gripsack_store::journal::REMOVED {
        live.is_none()
    } else {
        live.as_deref() == Some(intended.as_str())
    };
    if !landed {
        return Err(std::io::Error::other(format!(
            "{} did not reach its intended state (expected {}, found {})",
            dest.display(),
            intended,
            live.as_deref().unwrap_or("absent")
        )));
    }
    Ok(())
}

/// Open a destination's parent as a capability, creating parents for
/// a fresh destination first. Deploy's check-then-write paths pin
/// THIS inode: the drift hash, the journal capture, and the write
/// all resolve relative to it — a parent symlink swapped in after
/// `dest_resolves_into` ran cannot redirect the write (plan/0021
/// phase 2).
pub(crate) fn dest_capability(
    dest: &Path,
) -> std::io::Result<(gripsack_fs::Dir, std::path::PathBuf)> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let dir = gripsack_fs::open(parent)?;
    Ok((
        dir,
        std::path::PathBuf::from(dest.file_name().unwrap_or_default()),
    ))
}
/// never silently overwrite.
pub(crate) fn deploy_entry(
    out: &mut Vec<store::DeployedEntry>,
    module: &str,
    store_path: &Path,
    entry: &Entry,
    ctx: &Ctx,
    prev: Option<&store::ModuleState>,
    version: Option<&str>,
) -> Result<(String, ReportKind), ExecError> {
    let from = match version {
        // install keys substitute {version} (the locked tag) AND the
        // platform placeholders (0016 §D1) — same surface as verify
        Some(v) => gripsack_fetch::expand_platform(&entry.from).replace("{version}", v),
        None => gripsack_fetch::expand_platform(&entry.from),
    };
    // Entry content is the store payload — always. The publish step
    // stages every repo-referenced `from` into the store, so a store
    // miss means a stale store (e.g. a config tree that gained a file
    // under an unmoved pin): that is an integrity failure, never a
    // reason to reach into the repo checkout and deploy a path the
    // store never published.
    let source = store_path.join(&from);
    let dest = expand_home(&entry.to);
    let fail = |detail: String| ExecError::Step {
        module: module.to_string(),
        step: "deploy".into(),
        detail,
    };
    // A destination resolving INTO the env repo turns a deploy into a
    // delete: a symlinked ancestor dir (a leftover from another
    // provisioner) lands the write inside the checkout and the module
    // eats its own source. The repo is never a legitimate target.
    //
    // One exception: an `owned` destination that is ITSELF a symlink
    // into the repo — almost certainly an artifact an older gripsack
    // wrote when config deployed straight from the checkout. Owned
    // semantics replace the link (nothing is ever written THROUGH
    // it), so swapping it for a store link is safe and is the only
    // migration path forward; refusing here stranded every config
    // module that predates the store (first apply after upgrade,
    // forever, with an error that pointed at the module instead of
    // the stale link).
    let dest_is_symlink = dest
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink());
    let owned_replace_ok = matches!(entry.mode, Ownership::Owned) && dest_is_symlink;
    if dest_resolves_into(&dest, &ctx.repo) && !owned_replace_ok {
        let hint = if dest_is_symlink {
            "\n  hint: the destination is a symlink into the repo — likely left by an \
             older gripsack that deployed config from the checkout; remove it and \
             re-apply, or declare the entry `owned` so gripsack replaces it"
        } else {
            ""
        };
        return Err(fail(format!(
            "{} resolves inside the env repo ({}) — refusing to deploy into the source checkout{hint}",
            entry.to,
            ctx.repo.display()
        )));
    }
    // Expansion is total: a placeholder surviving to deploy means a
    // {version} with no locked tag or a substitution bug — never a
    // path worth linking
    if from.contains('{') {
        return Err(fail(format!(
            "{} still contains a placeholder after expansion (from {})",
            from, entry.from
        )));
    }
    if !source.exists() {
        // install={} keys are payload-relative — a versioned top-level
        // dir in the archive must be part of the key; say what IS here
        let hint = std::fs::read_dir(store_path)
            .map(|entries| {
                let names: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                if names.is_empty() {
                    String::new()
                } else {
                    format!(" (payload top-level: {})", names.join(", "))
                }
            })
            .unwrap_or_default();
        return Err(fail(format!(
            "no payload or repo file at {} (from {}){hint}",
            source.display(),
            entry.from
        )));
    }
    if source.is_dir() && entry.mode != Ownership::Owned {
        return Err(fail(format!(
            "{:?} on a directory ({}) — directory payloads are not supported yet; owned symlinks work today",
            entry.mode, entry.from
        )));
    }
    // template payloads render at deploy time — the vars were computed
    // by the frontend at eval; the core only substitutes (0001 §3.7)
    let rendered = match &entry.mode {
        Ownership::Template => Some(crate::template::render_template(
            &std::fs::read(&source)?,
            &entry.vars,
            &entry.from,
        )?),
        _ => None,
    };
    let (summary, kind, hash, prior, preserved_drift) = match &entry.mode {
        Ownership::Owned => {
            // external satisfaction (0009 critique): never overwrite a
            // path that is neither ours (symlink into the store) nor
            // recorded in a previous manifest — unless --take-over.
            let recorded = prev
                .map(|m| m.entries.iter().any(|e| e.to == entry.to))
                .unwrap_or(false);
            let ours = std::fs::read_link(&dest)
                .map(|t| t.starts_with(&ctx.home))
                .unwrap_or(false);
            let take = ctx.takes_over(&entry.to);
            // foreign symlinks refuse too (review finding E4): a stow/
            // chezmoi link is exactly the foreign path this guard is
            // for — absorb it only via --take-over, never silently
            if dest.symlink_metadata().is_ok() && !ours && !recorded && !take {
                return Err(ExecError::Step {
                    module: module.to_string(),
                    step: "deploy".into(),
                    detail: format!(
                        "{} exists and was not deployed by gripsack — move it away or use --take-over",
                        entry.to
                    ),
                });
            }
            // The dest-parent capability, opened where the
            // dest-resolves-into check ran: prior capture, the
            // journal, and the link swap pin ONE parent inode.
            let (dest_dir, dest_name) = dest_capability(&dest)?;
            // 0015 §4: a genuine take-over records what was there first
            let prior = if take && !ours && !recorded {
                capture_prior(&dest_dir, &dest_name, ctx.home_dir()?)?
            } else {
                None
            };
            // idempotent report (0014): a link already pointing at the
            // right store path is "unchanged", not "linked" — a mirror
            // swap that re-proves byte identity must not look like a
            // redeploy
            let already = std::fs::read_link(&dest)
                .map(|t| t == source)
                .unwrap_or(false);
            if !already {
                let target = source.to_string_lossy().into_owned();
                // precondition: the object we replace is the one the
                // guards inspected — link target for a symlink, content
                // hash for a take-over of a regular file
                let expected = match gripsack_store::journal::live_identity(&dest_dir, &dest_name)?
                {
                    Some(l) => Expect::Is(l),
                    None => Expect::Absent,
                };
                journaled(
                    ctx.home_dir()?,
                    &dest_dir,
                    &dest_name,
                    &dest,
                    target,
                    expected,
                    || gripsack_fs::symlink_replace(&dest_dir, &dest_name, &source),
                )?;
            }
            let hash = store::canonical_file_hash(&source)?;
            if already {
                (
                    format!("{} unchanged", entry.to),
                    ReportKind::Satisfied,
                    hash,
                    prior,
                    false,
                )
            } else {
                (
                    format!("linked {} → {}", from, entry.to),
                    ReportKind::Installed,
                    hash,
                    prior,
                    false,
                )
            }
        }
        Ownership::TrackedCopy | Ownership::Template => {
            let content;
            let owned_bytes;
            if let Some(r) = &rendered {
                content = r.as_slice();
            } else {
                owned_bytes = std::fs::read(&source)?;
                content = owned_bytes.as_slice();
            }
            let hash = match &rendered {
                Some(bytes) => store::canonical_bytes_hash(bytes),
                None => store::canonical_file_hash(&source)?,
            };
            let prev_pair = prev
                .and_then(|m| m.entries.iter().find(|e| e.to == entry.to))
                .map(|e| (e.hash.as_str(), e.preserved_drift));
            let (dest_dir, dest_name) = dest_capability(&dest)?;
            // branch on the OBJECT TYPE, never exists() (0029 §5):
            // exists() follows links — a dangling foreign link used to
            // read as "absent" and lose its identity
            let live = match dest_dir.symlink_metadata(&dest_name) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => {
                    return Err(fail(format!("cannot inspect {}: {e}", entry.to)));
                }
                Ok(m) if m.file_type().is_symlink() => {
                    // a symlink here is a foreign OBJECT (copy/template
                    // never write links) — its identity is its target
                    let target = dest_dir.read_link_contents(&dest_name)?;
                    Some(store::canonical_bytes_hash(
                        target.as_os_str().as_encoded_bytes(),
                    ))
                }
                Ok(m) if m.is_file() => Some(store::canonical_file_hash_in(&dest_dir, &dest_name)?),
                Ok(m) if m.is_dir() => {
                    return Err(fail(format!("{} is a directory", entry.to)));
                }
                Ok(_) => {
                    return Err(fail(format!(
                        "{} is not a regular file — refusing to touch it",
                        entry.to
                    )));
                }
            };
            // the journal's identity domain (link targets verbatim,
            // canonical bytes hash for files) — distinct from the
            // manifest domain (exec-aware file hash); the precondition
            // speaks the journal's
            let live_journal = store::journal::live_identity(&dest_dir, &dest_name)?;
            let expect = match &live_journal {
                Some(l) => Expect::Is(l.clone()),
                None => Expect::Absent,
            };
            match plan_copy(&hash, live.as_deref(), prev_pair, ctx.takes_over(&entry.to)) {
                CopyPlan::Satisfied => (
                    format!("{} unchanged", entry.to),
                    ReportKind::Satisfied,
                    hash,
                    None,
                    false,
                ),
                CopyPlan::Fresh | CopyPlan::Update => {
                    let update = live.is_some();
                    let after = store::canonical_bytes_hash(content);
                    journaled(
                        ctx.home_dir()?,
                        &dest_dir,
                        &dest_name,
                        &dest,
                        after,
                        expect.clone(),
                        || gripsack_fs::atomic_write(&dest_dir, &dest_name, content),
                    )?;
                    (
                        format!(
                            "{} {} → {}",
                            if update { "updated" } else { "copied" },
                            from,
                            entry.to
                        ),
                        ReportKind::Configured,
                        hash,
                        None,
                        false,
                    )
                }
                CopyPlan::TakeOver => {
                    // 0015 §4: record the foreign bytes before absorbing
                    let prior = capture_prior(&dest_dir, &dest_name, ctx.home_dir()?)?;
                    let after = store::canonical_bytes_hash(content);
                    journaled(
                        ctx.home_dir()?,
                        &dest_dir,
                        &dest_name,
                        &dest,
                        after,
                        expect.clone(),
                        || gripsack_fs::atomic_write(&dest_dir, &dest_name, content),
                    )?;
                    (
                        format!("took over {} → {}", from, entry.to),
                        ReportKind::Configured,
                        hash,
                        prior,
                        false,
                    )
                }
                CopyPlan::Preserve => {
                    let note = if prev_pair.is_none() {
                        format!("{} exists (not deployed by gripsack) — kept", entry.to)
                    } else {
                        format!("{} drifted — kept", entry.to)
                    };
                    tracing::warn!("{}", note);
                    // the record holds what we OBSERVED, marked
                    // preserved (0029 §2): it authorizes nothing —
                    // the next apply re-evaluates the drift fresh
                    (
                        note,
                        ReportKind::Warned,
                        live.expect("Preserve implies a live object"),
                        None,
                        true,
                    )
                }
            }
        }
        Ownership::Merge => {
            // the file is foreign — we own exactly one delimited block
            // inside it and regenerate that block wholesale (conda's
            // replace-not-merge: drift inside the markers self-heals)
            let payload = std::fs::read_to_string(&source)
                .map_err(|e| fail(format!("cannot read {}: {e}", source.display())))?;
            let block = payload.trim_end_matches('\n');
            let hash = store::canonical_bytes_hash(block.as_bytes());
            let dest_exists = dest.symlink_metadata().is_ok();
            let existing = match read_foreign_text(&dest) {
                Some(text) => text,
                None if !dest_exists => String::new(),
                // a binary or unreadable dest would be replaced
                // wholesale by the marker block — silent data loss
                None => {
                    return Err(fail(format!(
                        "cannot merge into {}: destination is not UTF-8 text — \
                         merge mode manages a block inside a text file",
                        dest.display()
                    )));
                }
            };
            let extracted = crate::template::extract_block(&existing, module);
            // a module owns EVERY block carrying its name: a duplicate
            // is accumulated state to reconcile, not a steady state —
            // otherwise a tampered second block is invisible to the
            // content-hash guarantee and only ever removed as a silent
            // side effect of the FIRST block drifting (0.21.1 review)
            let block_total = crate::template::find_blocks(&existing, module).len();
            let satisfied = block_total == 1
                && extracted
                    .as_deref()
                    .is_some_and(|c| store::canonical_bytes_hash(c.as_bytes()) == hash);
            if satisfied {
                (
                    format!("{} block unchanged", entry.to),
                    ReportKind::Satisfied,
                    hash,
                    None,
                    false,
                )
            } else {
                // the open marker's sha is the content hash at deploy
                // time — a mismatch means the block was hand-edited
                // since (visible from the file alone, no manifest
                // needed); the block regenerates either way
                let hand_edited = extracted.as_deref().is_some_and(|content| {
                    crate::template::marker_sha(&existing, module).is_some_and(|recorded| {
                        recorded != store::canonical_bytes_hash(content.as_bytes())[..16]
                    })
                });
                // whatever deploy does to a managed block, the report
                // names it (0.21.1 review): upsert regenerates the
                // first block and strips the duplicates behind it
                let mut notes = Vec::new();
                if hand_edited {
                    notes.push("hand-edited block regenerated".to_string());
                }
                if block_total > 1 {
                    notes.push(format!(
                        "removed {} duplicate block{}",
                        block_total - 1,
                        if block_total == 2 { "" } else { "s" }
                    ));
                }
                let note = if notes.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", notes.join(", "))
                };
                let new = crate::template::upsert_block(
                    &existing,
                    module,
                    &dest,
                    entry.marker.as_deref(),
                    &payload,
                )
                .map_err(fail)?;
                let (dest_dir, dest_name) = dest_capability(&dest)?;
                let after = store::canonical_bytes_hash(new.as_bytes());
                let expected = if dest_exists {
                    Expect::Is(store::canonical_bytes_hash(existing.as_bytes()))
                } else {
                    Expect::Absent
                };
                // merge re-derives from the LATEST foreign content at
                // the mutation boundary (0029 §3): an outside-block
                // write between the decision and here either lands in
                // the output or aborts the run — never silently lost
                let marker_owned = entry.marker.clone();
                let payload_owned = payload.clone();
                let module_owned = module.to_string();
                let dest_owned = dest.clone();
                journaled(
                    ctx.home_dir()?,
                    &dest_dir,
                    &dest_name,
                    &dest,
                    after,
                    expected,
                    || {
                        let latest = match dest_dir.read_to_string(&dest_name) {
                            Ok(t) => t,
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                            Err(e) => return Err(e),
                        };
                        if store::canonical_bytes_hash(latest.as_bytes())
                            != store::canonical_bytes_hash(existing.as_bytes())
                        {
                            return Err(std::io::Error::other(format!(
                                "{} changed between the merge decision and the write — aborting; re-run to retry",
                                dest_owned.display()
                            )));
                        }
                        let new = crate::template::upsert_block(
                            &latest,
                            &module_owned,
                            &dest_owned,
                            marker_owned.as_deref(),
                            &payload_owned,
                        )
                        .map_err(std::io::Error::other)?;
                        gripsack_fs::atomic_write(&dest_dir, &dest_name, new.as_bytes())
                    },
                )?;
                (
                    format!("merged {} → {}{note}", from, entry.to),
                    ReportKind::Configured,
                    hash,
                    None,
                    false,
                )
            }
        }
    };
    out.push(store::DeployedEntry {
        // the EXPANDED key — rollback (restore_entry) and store verify
        // re-join it against the store path verbatim; recording the
        // raw key would write placeholder-literal links on a rollback
        from: from.clone(),
        to: entry.to.clone(),
        mode: entry.mode.clone(),
        vars: entry.vars.clone(),
        hash,
        prior,
        preserved_drift,
    });
    Ok((summary, kind))
}

/// Does `dest` resolve inside `repo`? Canonicalize the deepest
/// existing ancestor — a symlinked intermediate directory resolves
/// THROUGH to its target — then re-append the not-yet-existing tail.
fn dest_resolves_into(dest: &Path, repo: &Path) -> bool {
    let Ok(repo_canon) = std::fs::canonicalize(repo) else {
        return false;
    };
    let mut ancestor = dest;
    while ancestor.symlink_metadata().is_err() {
        let Some(parent) = ancestor.parent() else {
            return false;
        };
        ancestor = parent;
    }
    let Ok(ancestor_canon) = std::fs::canonicalize(ancestor) else {
        return false;
    };
    let tail = dest.strip_prefix(ancestor).expect("ancestor is a prefix");
    ancestor_canon.join(tail).starts_with(&repo_canon)
}

/// Read a foreign (user-owned) destination as text: absent counts as
/// empty (merge creates the file); anything unreadable or non-UTF-8
/// is None — callers must refuse to splice onto it, never fall back
/// to "" and replace the file.
fn read_foreign_text(dest: &Path) -> Option<String> {
    match std::fs::read_to_string(dest) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(String::new()),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journaled_mutation_must_reach_its_intended_state() {
        // 0027 §1: a helper that returns Ok without producing the
        // intended state fails the run here — the flip never commits
        // an unverified destination
        let dir = tempfile::tempdir().unwrap();
        let home = gripsack_fs::open_or_create(dir.path()).unwrap();
        let dest = dir.path().join("config");
        let (dest_dir, dest_name) = dest_capability(&dest).unwrap();
        let err = journaled(
            &home,
            &dest_dir,
            &dest_name,
            &dest,
            store::canonical_bytes_hash(b"intended"),
            Expect::Absent,
            || Ok(()), // reports success, writes nothing
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("did not reach its intended state"),
            "{err}"
        );
        // the journal entry survives for reconcile
        let lines = store::journal::reconcile(&home).unwrap();
        assert!(!lines.is_empty());
    }

    #[test]
    fn restore_never_writes_a_dangling_owned_link() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store/abc-m");
        std::fs::create_dir_all(&store_path).unwrap();
        let dest = dir.path().join("home/.local/bin/m");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        // a stale manifest entry: a raw, unexpanded placeholder key
        // (what pre-fix generations recorded)
        let entry = store::DeployedEntry {
            from: "m-{version}-{target}/m".into(),
            to: dest.to_string_lossy().into_owned(),
            mode: Ownership::Owned,
            vars: Default::default(),
            hash: "x".repeat(64),
            prior: None,
            preserved_drift: false,
        };
        restore_entry(&dest, &entry, &store_path, "m").unwrap();
        assert!(
            dest.symlink_metadata().is_err(),
            "a missing restore source must leave the destination absent, not dangling"
        );
    }

    #[test]
    fn symlinked_ancestor_into_repo_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("dotfiles");
        std::fs::create_dir_all(repo.join(".claude/scripts")).unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        // the migration landmine: a leftover symlink pointing back
        // into the env repo
        std::os::unix::fs::symlink(repo.join(".claude/scripts"), home.join("scripts")).unwrap();

        // the repo path itself, and a not-yet-existing path under it
        assert!(dest_resolves_into(&repo.join("new/dir/file"), &repo));
        // ordinary destinations nowhere near the repo pass
        assert!(!dest_resolves_into(
            &home.join(".config/app/conf.toml"),
            &repo
        ));
        assert!(!dest_resolves_into(&home.join("scripts2/x"), &repo));
    }
}
