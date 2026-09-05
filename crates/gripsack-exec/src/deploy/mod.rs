//! Deploy: ownership modes, drift, destinations (0001 §3.7).

pub(crate) mod remove;
pub(crate) mod restore;

pub use remove::{remove_entry_deployed, remove_or_restore_prior};
use restore::capture_prior;
pub use restore::{
    RestorePlan, RestoreWrite, compute_restore, execute_restore, intact_deployed,
    intact_deployed_relative, prune_intent, restore_entry,
};

use crate::ctx::{Ctx, ExecError};
use crate::report::ReportKind;
use gripsack_ir::{Entry, Ownership};
use gripsack_store as store;
use std::path::Path;

/// no second observation can silently rebase the precondition.
enum Observation {
    File { bytes: Vec<u8>, mode: u32 },
    Symlink { target: std::ffi::OsString },
}

fn observe(dest_dir: &gripsack_fs::Dir, dest_name: &Path) -> std::io::Result<Option<Observation>> {
    let meta = match dest_dir.symlink_metadata(dest_name) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
        Ok(m) => m,
    };
    if meta.file_type().is_symlink() {
        let target = dest_dir.read_link_contents(dest_name)?;
        Ok(Some(Observation::Symlink {
            target: target.into_os_string(),
        }))
    } else if meta.is_file() {
        let mode = {
            use gripsack_fs::cap_std::fs::MetadataExt;
            meta.mode() & 0o7777
        };
        let bytes = dest_dir.read(dest_name)?;
        Ok(Some(Observation::File { bytes, mode }))
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file or symlink",
        ))
    }
}

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
/// One deployed destination's outcome: the report row plus the
/// facts the manifest entry records (0031: the landed mode among
/// them). A struct, not a six-slot tuple — the arms name their facts.
struct DeployOutcome {
    summary: String,
    kind: ReportKind,
    hash: String,
    /// The mode the destination holds after this deploy — tracked
    /// copies and templates only; links and merge blocks carry none
    /// (the link's target matters; the foreign file's mode is not
    /// ours).
    file_mode: Option<u32>,
    prior: Option<store::Prior>,
    preserved_drift: bool,
}

/// What the precondition expects of the live object before the
/// mutation — None: the destination must be ABSENT, anything
/// appearing aborts the run. (The 0031 typed form of the old
/// `Expect` enum: `Option<ObjectIdentity>`, no stringly `Is`.)
pub(crate) type Expect = Option<store::journal::ObjectIdentity>;

pub(crate) fn journaled(
    home: &gripsack_fs::Dir,
    dest_dir: &gripsack_fs::Dir,
    dest_name: &Path,
    dest: &Path,
    intended: store::journal::Intended,
    expected_before: Expect,
    mutate: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    use store::journal::{Intended, ObjectIdentity};
    // the live object must still be the one the drift decision was
    // made against — a write between decision and capture aborts
    // instead of clobbering it. (There is no portable content-CAS:
    // renameat2 RENAME_EXCHANGE is Linux-only. Capture and mutation
    // are back-to-back; the residual window is documented on the
    // safety page.)
    let live = gripsack_store::journal::live_identity(dest_dir, dest_name)?;
    if live != expected_before {
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
    let landed = match &intended {
        Intended::Removed => live.is_none(),
        Intended::Object(id) => live.as_ref() == Some(id),
    };
    if !landed {
        return Err(std::io::Error::other(format!(
            "{} did not reach its intended state (expected {}, found {})",
            dest.display(),
            intended.to_wire(),
            live.as_ref()
                .map(ObjectIdentity::to_wire)
                .as_deref()
                .unwrap_or("absent")
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
    prev_map: &std::collections::BTreeMap<std::path::PathBuf, &store::DeployedEntry>,
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
    // the canonical physical key (0030 §P0-1): one observation, one
    // transition, one journal key per physical object
    let dest = store::canonical_dest(&entry.to).map_err(|e| ExecError::Step {
        module: module.to_string(),
        step: "deploy".into(),
        detail: format!("destination {:?}: {e}", entry.to),
    })?;
    // lineage is destination-global: the previous generation's entry
    // for THIS physical destination, whichever module owned it
    let prev = prev_map.get(&dest).copied();
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
    let outcome = match &entry.mode {
        Ownership::Owned => {
            // external satisfaction (0009 critique): never overwrite a
            // path that is neither ours (symlink into the store) nor
            // recorded in a previous manifest — unless --take-over.
            let recorded = prev.is_some();
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
                // precondition: the object we replace is the one the
                // guards inspected — link target for a symlink, content
                // hash for a take-over of a regular file
                let expected: Expect =
                    gripsack_store::journal::live_identity(&dest_dir, &dest_name)?;
                journaled(
                    ctx.home_dir()?,
                    &dest_dir,
                    &dest_name,
                    &dest,
                    store::journal::Intended::Object(store::journal::ObjectIdentity::Link(
                        source.to_string_lossy().into_owned(),
                    )),
                    expected,
                    || gripsack_fs::symlink_replace(&dest_dir, &dest_name, &source),
                )?;
            }
            let hash = store::canonical_file_hash(&source)?.to_string();
            if already {
                DeployOutcome {
                    summary: format!("{} unchanged", entry.to),
                    kind: ReportKind::Satisfied,
                    hash,
                    file_mode: None,
                    prior,
                    preserved_drift: false,
                }
            } else {
                DeployOutcome {
                    summary: format!("linked {} → {}", from, entry.to),
                    kind: ReportKind::Installed,
                    hash,
                    file_mode: None,
                    prior,
                    preserved_drift: false,
                }
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
            // the intended LANDED mode (0031): tracked copies manage
            // executability — 0755 when the payload is exec, 0644
            // otherwise. The identity is computed from INTENT, never
            // from the store payload's own mode (read-only 0555/0444
            // would hash under the extended preimage)
            #[cfg(unix)]
            let src_exec = {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(&source)?.permissions().mode() & 0o111 != 0
            };
            #[cfg(not(unix))]
            let src_exec = false;
            let intent_mode: u32 = if src_exec { 0o755 } else { 0o644 };
            // the modal manifest identity (the field is a wire string;
            // the TYPE of the hash is fixed by the entry's mode, so a
            // cross-mode comparison is impossible by construction)
            let hash = match entry.mode {
                // templates: bytes-only identity — a rendered file's
                // mode is not managed (0030 §H3)
                Ownership::Template => store::canonical_bytes_hash(content).to_string(),
                _ => store::canonical_bytes_identity(content, intent_mode).to_string(),
            };
            let prev_pair = prev.map(|e| (e.hash.as_str(), e.preserved_drift));
            let (dest_dir, dest_name) = dest_capability(&dest)?;
            // ONE observation drives the decision AND the precondition
            // (0030 §P0-2 — the 0029 code read twice, and the second
            // read silently became the authorized baseline). Both
            // identity domains derive from the same bytes; the
            // journal's file identity is mode-aware (0031).
            let observed: Option<Observation> = observe(&dest_dir, &dest_name)?;
            // both identity domains from the ONE observation:
            // manifest (plan_copy) is mode-specific — tracked copies
            // are mode-aware (0031: chmod-only drift is drift),
            // templates bytes-only, links hash their target bytes; the
            // journal/precondition domain is mode-aware for files,
            // raw target for links
            let (live, expect): (Option<String>, Expect) = match &observed {
                None => (None, None),
                Some(Observation::Symlink { target }) => (
                    Some(store::canonical_bytes_hash(target.as_encoded_bytes()).to_string()),
                    Some(store::journal::ObjectIdentity::Link(
                        target.to_string_lossy().into_owned(),
                    )),
                ),
                Some(Observation::File { bytes, mode }) => {
                    let manifest = match entry.mode {
                        Ownership::Template => store::canonical_bytes_hash(bytes).to_string(),
                        _ => store::canonical_bytes_identity(bytes, *mode).to_string(),
                    };
                    (
                        Some(manifest),
                        Some(store::journal::ObjectIdentity::File(
                            store::canonical_bytes_identity(bytes, *mode),
                        )),
                    )
                }
            };
            match plan_copy(&hash, live.as_deref(), prev_pair, ctx.takes_over(&entry.to)) {
                CopyPlan::Satisfied => DeployOutcome {
                    summary: format!("{} unchanged", entry.to),
                    kind: ReportKind::Satisfied,
                    hash,
                    // the recorded mode persists across a satisfied
                    // apply (the live object matches it by definition)
                    file_mode: prev.and_then(|e| e.file_mode),
                    prior: None,
                    preserved_drift: false,
                },
                CopyPlan::Fresh | CopyPlan::Update => {
                    let update = live.is_some();
                    // the mode the write will land (0031): tracked
                    // copies get their intended mode; templates
                    // preserve the existing file's mode on update
                    // (0026 §7) and land 0644 fresh — deterministic,
                    // so the journaled precondition never depends on
                    // the process umask
                    let landed_mode: u32 = match entry.mode {
                        Ownership::Template => match &observed {
                            Some(Observation::File { mode, .. }) => *mode,
                            _ => 0o644,
                        },
                        _ => intent_mode,
                    };
                    let after =
                        store::journal::Intended::Object(store::journal::ObjectIdentity::File(
                            store::canonical_bytes_identity(content, landed_mode),
                        ));
                    journaled(
                        ctx.home_dir()?,
                        &dest_dir,
                        &dest_name,
                        &dest,
                        after,
                        expect.clone(),
                        || {
                            if entry.mode == Ownership::Template && update {
                                // mode preserved — see landed_mode
                                gripsack_fs::atomic_write(&dest_dir, &dest_name, content)
                            } else {
                                // one atomic write lands bytes AND
                                // mode — fresh or update, the mode is
                                // what the source declares
                                gripsack_fs::atomic_write_with_mode(
                                    &dest_dir,
                                    &dest_name,
                                    content,
                                    landed_mode,
                                )
                            }
                        },
                    )?;
                    DeployOutcome {
                        summary: format!(
                            "{} {} → {}",
                            if update { "updated" } else { "copied" },
                            from,
                            entry.to
                        ),
                        kind: ReportKind::Configured,
                        hash,
                        file_mode: Some(landed_mode),
                        prior: None,
                        preserved_drift: false,
                    }
                }
                CopyPlan::TakeOver => {
                    // 0015 §4: record the foreign bytes before absorbing
                    let prior = capture_prior(&dest_dir, &dest_name, ctx.home_dir()?)?;
                    // the absorbed file is OURS now — it lands the
                    // managed mode (0031), matching the recorded
                    // identity exactly
                    let after =
                        store::journal::Intended::Object(store::journal::ObjectIdentity::File(
                            store::canonical_bytes_identity(content, intent_mode),
                        ));
                    journaled(
                        ctx.home_dir()?,
                        &dest_dir,
                        &dest_name,
                        &dest,
                        after,
                        expect.clone(),
                        || {
                            gripsack_fs::atomic_write_with_mode(
                                &dest_dir,
                                &dest_name,
                                content,
                                intent_mode,
                            )
                        },
                    )?;
                    DeployOutcome {
                        summary: format!("took over {} → {}", from, entry.to),
                        kind: ReportKind::Configured,
                        hash,
                        file_mode: Some(intent_mode),
                        prior,
                        preserved_drift: false,
                    }
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
                    DeployOutcome {
                        summary: note,
                        kind: ReportKind::Warned,
                        hash: live.expect("Preserve implies a live object"),
                        file_mode: None,
                        prior: None,
                        preserved_drift: true,
                    }
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
            let hash = store::canonical_bytes_hash(block.as_bytes()).to_string();
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
                    .is_some_and(|c| store::canonical_bytes_hash(c.as_bytes()).as_str() == hash);
            if satisfied {
                DeployOutcome {
                    summary: format!("{} block unchanged", entry.to),
                    kind: ReportKind::Satisfied,
                    hash,
                    file_mode: None,
                    prior: None,
                    preserved_drift: false,
                }
            } else {
                // the open marker's sha is the content hash at deploy
                // time — a mismatch means the block was hand-edited
                // since (visible from the file alone, no manifest
                // needed); the block regenerates either way
                let hand_edited = extracted.as_deref().is_some_and(|content| {
                    crate::template::marker_sha(&existing, module).is_some_and(|recorded| {
                        recorded != store::canonical_bytes_hash(content.as_bytes()).as_str()[..16]
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
                // the journal speaks mode-aware identities (0031) —
                // merge preserves an existing file's mode; a fresh
                // merge-created file lands 0644 deterministically
                #[cfg(unix)]
                let dest_mode: u32 = std::fs::metadata(&dest)
                    .map(|m| {
                        use std::os::unix::fs::MetadataExt;
                        m.mode() & 0o7777
                    })
                    .unwrap_or(0o644);
                #[cfg(not(unix))]
                let dest_mode: u32 = 0o644;
                let after = store::journal::Intended::Object(store::journal::ObjectIdentity::File(
                    store::canonical_bytes_identity(new.as_bytes(), dest_mode),
                ));
                let expected: Expect = if dest_exists {
                    Some(store::journal::ObjectIdentity::File(
                        store::canonical_bytes_identity(existing.as_bytes(), dest_mode),
                    ))
                } else {
                    None
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
                        #[cfg(unix)]
                        let latest_mode = dest_dir
                            .metadata(&dest_name)
                            .map(|m| {
                                use gripsack_fs::cap_std::fs::MetadataExt;
                                m.mode() & 0o7777
                            })
                            .unwrap_or(0o644);
                        #[cfg(not(unix))]
                        let latest_mode = 0o644;
                        if store::canonical_bytes_identity(latest.as_bytes(), latest_mode)
                            != store::canonical_bytes_identity(existing.as_bytes(), dest_mode)
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
                        if dest_exists {
                            // the foreign file's mode is preserved
                            gripsack_fs::atomic_write(&dest_dir, &dest_name, new.as_bytes())
                        } else {
                            gripsack_fs::atomic_write_with_mode(
                                &dest_dir,
                                &dest_name,
                                new.as_bytes(),
                                0o644,
                            )
                        }
                    },
                )?;
                DeployOutcome {
                    summary: format!("merged {} → {}{note}", from, entry.to),
                    kind: ReportKind::Configured,
                    hash,
                    file_mode: None,
                    prior: None,
                    preserved_drift: false,
                }
            }
        }
    };
    let DeployOutcome {
        summary,
        kind,
        hash,
        file_mode,
        prior,
        preserved_drift,
    } = outcome;
    out.push(store::DeployedEntry {
        // the EXPANDED key — rollback (restore_entry) and store verify
        // re-join it against the store path verbatim; recording the
        // raw key would write placeholder-literal links on a rollback
        from: from.clone(),
        to: entry.to.clone(),
        mode: entry.mode.clone(),
        vars: entry.vars.clone(),
        hash,
        file_mode,
        prior,
        preserved_drift,
    });
    Ok((summary, kind))
}

/// Does `dest` resolve inside `repo`? Canonicalize the deepest
/// existing ancestor — a symlinked intermediate directory resolves
/// THROUGH to its target — then re-append the not-yet-existing tail.
pub(crate) fn dest_resolves_into(dest: &Path, repo: &Path) -> bool {
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
pub(crate) fn read_foreign_text(dest: &Path) -> Option<String> {
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
            store::journal::Intended::Object(store::journal::ObjectIdentity::Link(
                "intended".into(),
            )),
            None,
            || Ok(()), // reports success, writes nothing
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("did not reach its intended state"),
            "{err}"
        );
        // the journal entry survives for reconcile
        let lines = store::journal::reconcile(&home, dir.path()).unwrap();
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
            file_mode: None,
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
