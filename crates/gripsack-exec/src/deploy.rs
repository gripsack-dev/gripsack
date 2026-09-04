//! Deploy: ownership modes, drift, destinations (0001 §3.7).

use crate::ctx::{Ctx, ExecError};
use crate::report::ReportKind;
use gripsack_ir::{Entry, Ownership};
use gripsack_store as store;
use gripsack_store::expand_home;
use std::path::Path;

/// Restore one destination to what a generation's manifest recorded —
/// the ONE deploy-restore path, shared by run-level rollback and
/// `grip rollback` (0001 §3.5): every mode gets its correct semantics,
/// never a naive byte copy (template re-renders with the recorded
/// vars; merge re-upserts only the block into the foreign file).
pub fn restore_entry(
    dest: &Path,
    entry: &store::DeployedEntry,
    store_path: &Path,
    module: &str,
) -> std::io::Result<()> {
    let source = store_path.join(&entry.from);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match entry.mode {
        Ownership::Owned => {
            // never write a dangling link: a missing source means the
            // manifest is stale, and a broken symlink is worse than
            // an absent destination
            if !source.exists() {
                tracing::warn!(
                    ?source,
                    "restore source missing — leaving destination as-is"
                );
                return Ok(());
            }
            let (dest_dir, dest_name) = dest_capability(dest)?;
            gripsack_fs::symlink_replace(&dest_dir, &dest_name, &source)
        }
        Ownership::Merge => {
            let payload = std::fs::read_to_string(&source).unwrap_or_default();
            // a dest that is not text cannot host a managed block:
            // splicing onto "" would REPLACE the whole foreign file
            // (silent data loss) — leave it alone instead
            let Some(existing) = read_foreign_text(dest) else {
                return Ok(());
            };
            match crate::template::upsert_block(&existing, module, dest, None, &payload) {
                Ok(new) => {
                    let (dest_dir, dest_name) = dest_capability(dest)?;
                    gripsack_fs::atomic_write(&dest_dir, &dest_name, new.as_bytes())
                }
                Err(_) => Ok(()), // malformed markers: leave the foreign file alone
            }
        }
        Ownership::Template => std::fs::read(&source).and_then(|raw| {
            crate::template::render_template(&raw, &entry.vars, &entry.from)
                .map_err(std::io::Error::other)
                .and_then(|bytes| {
                    let (dest_dir, dest_name) = dest_capability(dest)?;
                    gripsack_fs::atomic_write(&dest_dir, &dest_name, &bytes)
                })
        }),
        Ownership::TrackedCopy => std::fs::read(&source).and_then(|bytes| {
            let (dest_dir, dest_name) = dest_capability(dest)?;
            gripsack_fs::atomic_write(&dest_dir, &dest_name, &bytes)
        }),
    }
}

/// Remove a destination we deployed, with drift guards (0001 §3.5):
/// never delete user edits. Returns true if anything was removed.
/// Merge entries remove only our block from the foreign file.
pub fn remove_entry_deployed(dest: &Path, entry: &store::DeployedEntry, module: &str) -> bool {
    match entry.mode {
        Ownership::Owned => {
            let ours = std::fs::read_link(dest)
                .map(|t| t.starts_with(store::gripsack_home()))
                .unwrap_or(false);
            ours && std::fs::remove_file(dest).is_ok()
        }
        Ownership::Merge => {
            let existing = std::fs::read_to_string(dest).unwrap_or_default();
            match crate::template::extract_block(&existing, module) {
                Some(content) if store::canonical_bytes_hash(content.as_bytes()) == entry.hash => {
                    let new = crate::template::remove_block(&existing, module)
                        .expect("block found above");
                    if new.trim().is_empty() {
                        std::fs::remove_file(dest).is_ok()
                    } else {
                        dest_capability(dest)
                            .and_then(|(d, n)| gripsack_fs::atomic_write(&d, &n, new.as_bytes()))
                            .is_ok()
                    }
                }
                _ => false, // drifted block is the user's now
            }
        }
        _ => {
            // copy-like: only delete bytes identical to what we wrote
            let Ok(current) = store::canonical_file_hash(dest) else {
                return false;
            };
            current == entry.hash && std::fs::remove_file(dest).is_ok()
        }
    }
}

/// Record what a destination is before a take-over absorbs it (0015
/// §4): real-file bytes go to the content-addressed prior blob store,
/// a symlink's target is recorded verbatim. None = nothing there (or
/// unreadable) — default removal semantics then apply.
fn capture_prior(
    dest_dir: &gripsack_fs::Dir,
    dest_name: &Path,
    home: &gripsack_fs::Dir,
) -> Option<store::Prior> {
    let meta = dest_dir.symlink_metadata(dest_name).ok()?;
    if meta.file_type().is_symlink() {
        let target = dest_dir.read_link_contents(dest_name).ok()?;
        Some(store::Prior {
            kind: store::PriorKind::Symlink,
            content: Some(target.to_string_lossy().into_owned()),
            mode: None,
        })
    } else if meta.is_file() {
        let bytes = dest_dir.read(dest_name).ok()?;
        let sha = store::journal::store_prior_blob_in(home, &bytes).ok()?;
        #[cfg(unix)]
        let mode = {
            use gripsack_fs::cap_std::fs::MetadataExt;
            Some(meta.mode() & 0o777)
        };
        #[cfg(not(unix))]
        let mode = None;
        Some(store::Prior {
            kind: store::PriorKind::File,
            content: Some(sha),
            mode,
        })
    } else {
        None
    }
}

/// Write a prior state back to its destination (0015 §4).
fn restore_prior(dest: &Path, prior: &store::Prior, home: &Path) -> bool {
    match prior.kind {
        store::PriorKind::File => {
            let Some(sha) = &prior.content else {
                return false;
            };
            let Ok(bytes) = std::fs::read(store::prior_blob_path(home, sha)) else {
                return false;
            };
            if !dest_capability(dest)
                .and_then(|(d, n)| gripsack_fs::atomic_write(&d, &n, &bytes))
                .is_ok()
            {
                return false;
            }
            #[cfg(unix)]
            if let Some(mode) = prior.mode {
                use std::os::unix::fs::PermissionsExt;
                return std::fs::set_permissions(dest, std::fs::Permissions::from_mode(mode))
                    .is_ok();
            }
            true
        }
        store::PriorKind::Symlink => {
            let Some(target) = &prior.content else {
                return false;
            };
            // symlink_replace over remove+create: the swap is atomic
            // and parent-fsync'd (strictly stronger than the old pair)
            dest_capability(dest)
                .and_then(|(d, n)| gripsack_fs::symlink_replace(&d, &n, Path::new(target)))
                .is_ok()
        }
    }
}

/// Rollback/prune for a deployed entry (0015 §4): when the destination
/// is still exactly what gripsack deployed and a prior exists, restore
/// the original file/symlink — "your original files have been
/// restored." Drifted destinations and prior-less entries fall back to
/// the drift-guarded removal.
pub fn remove_or_restore_prior(
    dest: &Path,
    entry: &store::DeployedEntry,
    module: &str,
    home: &Path,
) -> bool {
    let intact = match entry.mode {
        Ownership::Owned => std::fs::read_link(dest)
            .map(|t| t.starts_with(home))
            .unwrap_or(false),
        Ownership::Merge => false, // merge never carries a prior
        _ => store::canonical_file_hash(dest)
            .map(|h| h == entry.hash)
            .unwrap_or(false),
    };
    if intact && let Some(prior) = &entry.prior {
        return restore_prior(dest, prior, home);
    }
    remove_entry_deployed(dest, entry, module)
}

/// Run-level rollback (0001 §9, review finding E1): an apply that
/// fails mid-graph must leave NO half-applied deployment behind —
/// the generation flip was never reached, so every destination this
/// run touched is restored to the previous generation's state (or
/// removed if the previous generation didn't deploy it; entries this
/// run absorbed via --take-over restore their captured prior first,
/// 0015 §4 — removing the deployed link alone would lose the user's
/// original file, whose bytes only exist in the prior blob store).
pub(crate) fn run_rollback(
    touched: &std::collections::BTreeMap<String, store::ModuleState>,
    prev: &std::collections::BTreeMap<String, store::ModuleState>,
    home: &Path,
) {
    for (name, state) in touched {
        let prev_state = prev.get(name);
        for entry in &state.entries {
            let dest = expand_home(&entry.to);
            match prev_state.and_then(|p| p.entries.iter().find(|e| e.to == entry.to)) {
                Some(prev_entry) => {
                    let result = restore_entry(
                        &dest,
                        prev_entry,
                        &prev_state.expect("matched above").store_path,
                        name,
                    );
                    if result.is_ok() {
                        tracing::info!("restored {} (run rolled back)", entry.to);
                    }
                }
                None => {
                    if remove_or_restore_prior(&dest, entry, name, home) {
                        tracing::info!("restored {} (run rolled back)", entry.to);
                    }
                }
            }
        }
    }
}

/// One destination mutation, journaled for crash recovery (0019):
/// the prior state is recorded (file bytes into the prior blob
/// store) BEFORE the write and the post-mutation identity noted
/// after — a kill anywhere in between leaves an uncommitted entry
/// the next run's reconcile restores. The entry clears when the run
/// commits (the flip); per-entry there is no commit, matching the
/// run-level rollback's all-or-nothing semantics.
fn journaled(
    home: &gripsack_fs::Dir,
    dest_dir: &gripsack_fs::Dir,
    dest_name: &Path,
    dest: &Path,
    after: String,
    mutate: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let prior = gripsack_store::journal::capture(dest_dir, dest_name, dest, home)?;
    gripsack_store::journal::record(home, dest, &prior)?;
    mutate()?;
    gripsack_store::journal::mark_after(home, dest, &after)
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
    let (summary, kind, hash, prior) = match &entry.mode {
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
                capture_prior(&dest_dir, &dest_name, ctx.home_dir()?)
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
                journaled(
                    ctx.home_dir()?,
                    &dest_dir,
                    &dest_name,
                    &dest,
                    target,
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
                )
            } else {
                (
                    format!("linked {} → {}", from, entry.to),
                    ReportKind::Installed,
                    hash,
                    prior,
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
            let prev_hash = prev
                .and_then(|m| m.entries.iter().find(|e| e.to == entry.to))
                .map(|e| e.hash.as_str());
            if dest.exists() {
                // drift check and any write below share the pinned
                // dest parent (plan/0021 phase 2)
                let (dest_dir, dest_name) = dest_capability(&dest)?;
                let current = store::canonical_file_hash_in(&dest_dir, &dest_name)?;
                if current == hash {
                    (
                        format!("{} unchanged", entry.to),
                        ReportKind::Satisfied,
                        hash,
                        None,
                    )
                } else if prev_hash == Some(current.as_str()) {
                    let after = store::canonical_bytes_hash(content);
                    journaled(ctx.home_dir()?, &dest_dir, &dest_name, &dest, after, || {
                        gripsack_fs::atomic_write(&dest_dir, &dest_name, content)
                    })?;
                    (
                        format!("updated {} → {}", from, entry.to),
                        ReportKind::Configured,
                        hash,
                        None,
                    )
                } else if ctx.takes_over(&entry.to) {
                    // 0015 §4: record the foreign bytes before absorbing
                    let prior = capture_prior(&dest_dir, &dest_name, ctx.home_dir()?);
                    let after = store::canonical_bytes_hash(content);
                    journaled(ctx.home_dir()?, &dest_dir, &dest_name, &dest, after, || {
                        gripsack_fs::atomic_write(&dest_dir, &dest_name, content)
                    })?;
                    (
                        format!("took over {} → {}", from, entry.to),
                        ReportKind::Configured,
                        hash,
                        prior,
                    )
                } else {
                    let note = if prev_hash.is_none() {
                        format!("{} exists (not deployed by gripsack) — kept", entry.to)
                    } else {
                        format!("{} drifted — kept", entry.to)
                    };
                    tracing::warn!("{}", note);
                    // the manifest must record what's ACTUALLY deployed
                    // (the kept content), not the source we declined to
                    // write — or drift resolution can never converge
                    // (e2e: drift_is_kept)
                    (note, ReportKind::Warned, current, None)
                }
            } else {
                let (dest_dir, dest_name) = dest_capability(&dest)?;
                let after = store::canonical_bytes_hash(content);
                journaled(ctx.home_dir()?, &dest_dir, &dest_name, &dest, after, || {
                    gripsack_fs::atomic_write(&dest_dir, &dest_name, content)
                })?;
                (
                    format!("copied {} → {}", from, entry.to),
                    ReportKind::Configured,
                    hash,
                    None,
                )
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
            let existing = match read_foreign_text(&dest) {
                Some(text) => text,
                None if !dest.exists() => String::new(),
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
            let satisfied = extracted
                .as_deref()
                .is_some_and(|c| store::canonical_bytes_hash(c.as_bytes()) == hash);
            if satisfied {
                (
                    format!("{} block unchanged", entry.to),
                    ReportKind::Satisfied,
                    hash,
                    None,
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
                let note = if hand_edited {
                    " (hand-edited block regenerated)"
                } else {
                    ""
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
                journaled(ctx.home_dir()?, &dest_dir, &dest_name, &dest, after, || {
                    gripsack_fs::atomic_write(&dest_dir, &dest_name, new.as_bytes())
                })?;
                (
                    format!("merged {} → {}{note}", from, entry.to),
                    ReportKind::Configured,
                    hash,
                    None,
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
