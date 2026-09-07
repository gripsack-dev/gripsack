//! Deploy: ownership modes, drift, destinations (0001 §3.7).

pub(crate) mod remove;
pub(crate) mod restore;

pub use remove::{remove_entry_deployed, remove_or_restore_prior};
pub(crate) use restore::intact_deployed;

use crate::ctx::{Ctx, ExecError};
use crate::report::ReportKind;
use gripsack_ir::{Entry, Ownership};
use gripsack_store as store;
use std::path::Path;

/// no second observation can silently rebase the precondition.
pub enum Observation {
    File { bytes: Vec<u8>, mode: u32 },
    Symlink { target: std::ffi::OsString },
}

pub(crate) fn observe(
    dest_dir: &gripsack_fs::Dir,
    dest_name: &Path,
) -> std::io::Result<Option<Observation>> {
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

/// The owned-link disposition as a pure function (0033 R7) — the
/// lineage explorer drives THIS, like plan_copy for copies. `exists`:
/// anything at the destination; `ours`: a link into the store;
/// `recorded`: a previous manifest entry that is NOT preserved drift
/// (preserved drift authorizes nothing, including a mode change).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkPlan {
    /// Nothing there (or already ours to redeploy): point the link.
    Link,
    /// Absorb the foreign object --take-over (a prior is captured).
    TakeOver,
    /// A foreign object blocks the deploy — move it or --take-over.
    Refuse,
}

pub(crate) fn plan_link(exists: bool, ours: bool, recorded: bool, take_over: bool) -> LinkPlan {
    if exists && !ours && !recorded && !take_over {
        LinkPlan::Refuse
    } else if take_over && !ours && !recorded {
        LinkPlan::TakeOver
    } else {
        LinkPlan::Link
    }
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

/// A read-only observation by plain path (0035 F7): the preview's
/// eyes — no capability, NO directory creation. Mutation paths use
/// `observe` through the pinned parent.
pub fn observe_readonly(dest: &Path) -> std::io::Result<Option<Observation>> {
    let meta = match std::fs::symlink_metadata(dest) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
        Ok(m) => m,
    };
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(dest)?;
        Ok(Some(Observation::Symlink {
            target: target.into_os_string(),
        }))
    } else if meta.is_file() {
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::MetadataExt;
            meta.mode() & 0o7777
        };
        #[cfg(not(unix))]
        let mode = 0o644;
        let bytes = std::fs::read(dest)?;
        Ok(Some(Observation::File { bytes, mode }))
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file or symlink",
        ))
    }
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
    let prepared = crate::source::payload_source(store_path, &entry.from, version);
    let from = prepared.relative;
    // Entry content is the store payload — always. The publish step
    // stages every repo-referenced `from` into the store, so a store
    // miss means a stale store (e.g. a config tree that gained a file
    // under an unmoved pin): that is an integrity failure, never a
    // reason to reach into the repo checkout and deploy a path the
    // store never published.
    let source = prepared.path;
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
    // the ONE planner (0034): the op IS the decision — plan renders
    // it, apply executes it, rollback constructs it
    let view = crate::ops::DestView {
        module,
        entry,
        dest: dest.clone(),
        home: &ctx.home,
        observed: {
            let (dest_dir, dest_name) = dest_capability(&dest)?;
            observe(&dest_dir, &dest_name)?
        },
        prev,
        take_over: ctx.takes_over(&entry.to),
    };
    let op = match &entry.mode {
        Ownership::Owned => {
            let already = std::fs::read_link(&dest)
                .map(|t| t == source)
                .unwrap_or(false);
            crate::ops::plan_entry_op(
                &view,
                crate::ops::ModeInput::Link {
                    source: &source,
                    content_hash: store::canonical_file_hash(&source)?.into(),
                    already,
                },
            )?
        }
        Ownership::TrackedCopy | Ownership::Template => {
            let content: &[u8] = match &rendered {
                Some(r) => r.as_slice(),
                None => &std::fs::read(&source)?,
            };
            // Whole-file outputs share the same source-executability policy.
            #[cfg(unix)]
            let src_exec = {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(&source)?.permissions().mode() & 0o111 != 0
            };
            #[cfg(not(unix))]
            let src_exec = false;
            crate::ops::plan_entry_op(
                &view,
                crate::ops::ModeInput::Write {
                    content,
                    permissions: crate::ops::WritePermissions::Source {
                        executable: src_exec,
                    },
                },
            )?
        }
        Ownership::Merge => {
            let payload = std::fs::read_to_string(&source)
                .map_err(|e| fail(format!("cannot read {}: {e}", source.display())))?;
            let existing = read_foreign_text(&dest);
            crate::ops::plan_entry_op(
                &view,
                crate::ops::ModeInput::Merge {
                    payload: &payload,
                    existing,
                    permissions: crate::ops::WritePermissions::Preserve,
                },
            )?
        }
    };
    // a foreign destination blocks apply (the renderer shows the same
    // op as "needs --take-over")
    if op.authority == Some(crate::ops::Authority::Foreign) {
        return Err(ExecError::Step {
            module: module.to_string(),
            step: "deploy".into(),
            detail: format!(
                "{} exists and was not deployed by gripsack — move it away or use --take-over",
                entry.to
            ),
        });
    }
    let (report, captured_prior) = crate::ops::execute_op(ctx.home_dir()?, &ctx.home, &op)?;
    // the manifest entry: what the op produces, or the previous entry
    // carried forward (satisfied)
    match op.produces {
        Some(produced) => {
            out.push(store::DeployedEntry {
                // the EXPANDED key — rollback and store verify re-join
                // it against the store path verbatim
                from: std::path::PathBuf::from(&from),
                to: entry.to.clone(),
                key: Some(dest.clone()),
                mode: entry.mode.clone(),
                vars: entry.vars.clone(),
                hash: produced.hash,
                file_mode: produced.file_mode,
                source_executable: produced.source_executable,
                prior: captured_prior.or(produced.prior),
                preserved_drift: produced.preserved_drift,
            });
        }
        None => {
            if let Some(prev_entry) = prev {
                out.push((*prev_entry).clone());
            }
        }
    }
    Ok((report.summary, report.kind))
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
            key: None,
            mode: Ownership::Owned,
            vars: Default::default(),
            file_mode: None,
            source_executable: None,
            hash: gripsack_store::hash::ManifestHash::from_raw("x".repeat(64)),
            prior: None,
            preserved_drift: false,
        };
        // 0034: the planner answers None — no safe restore, no write
        let op = crate::ops::plan_restore_op("m", &entry, &store_path, None, dir.path()).unwrap();
        assert!(
            op.is_none(),
            "a missing restore source must plan NOTHING, not a dangling link"
        );
        assert!(
            dest.symlink_metadata().is_err(),
            "and the destination stays absent"
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
