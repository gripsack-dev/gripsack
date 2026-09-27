//! Deploy: ownership modes, drift, destinations (0001 §3.7).

mod entry;
pub(crate) use entry::{DeploymentInput, deploy_entry};
pub(crate) mod remove;
pub(crate) mod restore;

pub use remove::{remove_entry_deployed, remove_or_restore_prior};
pub(crate) use restore::intact_deployed;

#[cfg(test)]
use gripsack_ir::Ownership;
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

/// The copy/link authority kernels live in gripsack-policy (0046):
/// ONE implementation — production planning, the lineage explorer
/// (which drives these through materialized filesystem states) and
/// the Verus proofs share it. The authority rules (0029 §2, 0033 R7):
/// preserved drift NEVER promotes to authority — reconvergence
/// (live == desired) is the only way back; a managed update requires
/// agreement with the last managed write; explicit take-over always
/// absorbs, capturing the origin even when the bytes already match.
pub(crate) use gripsack_policy::ownership::{CopyPlan, LinkPlan, plan_copy, plan_link};

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
    use store::journal::Intended;
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
            intended,
            live.as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "absent".into())
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
            ownership: store::StoredOwnership::Legacy(Ownership::Owned),
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
