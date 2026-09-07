//! Prior capture/restore and intactness (0015 §4, 0029 §3): the
//! drift guards the ops planner relies on — the rollback restore path
//! lives in ops::plan (0034).

use gripsack_ir::Ownership;
use gripsack_store as store;
use std::path::Path;

/// Capture a destination's current state for a take-over (0015
/// §4): real-file bytes go to the content-addressed prior blob store,
/// a symlink's target is recorded verbatim. None = nothing there (or
/// unreadable) — default removal semantics then apply.
/// Strictly fallible (0025 §E): only NotFound means "no prior".
/// Every other read, metadata, encoding, or blob-storage failure
/// aborts the take-over BEFORE the mutation — recording `prior: None`
/// for a file that existed but could not be captured would break the
/// central promise (exact pre-adoption restoration).
pub(crate) fn capture_prior(
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
        Ok(Some(store::Prior::Symlink {
            target: target.to_string(),
        }))
    } else if meta.is_file() {
        let bytes = dest_dir.read(dest_name)?;
        let hash = store::journal::store_prior_blob_in(home, &bytes)?;
        #[cfg(unix)]
        let mode = {
            use gripsack_fs::cap_std::fs::MetadataExt;
            meta.mode() & 0o7777
        };
        #[cfg(not(unix))]
        let mode = 0o644;
        Ok(Some(store::Prior::File { hash, mode }))
    } else {
        Ok(None)
    }
}

/// Write a prior state back to its destination (0015 §4). Every
/// failure is an error (0027 §1): a prior that cannot be read,
/// written, or chmod'd must abort the transaction — the bool era read
/// those as "kept", committing a generation over a failed restore.
/// The recorded mode rides the write exactly (0027 §6).
pub(crate) fn restore_prior(
    dest_dir: &gripsack_fs::Dir,
    dest_name: &Path,
    prior: &store::Prior,
    home: &Path,
) -> std::io::Result<()> {
    match prior {
        store::Prior::File { hash, mode } => {
            let bytes = std::fs::read(store::prior_blob_path(home, hash))?;
            #[cfg(unix)]
            gripsack_fs::atomic_write_with_mode(dest_dir, dest_name, &bytes, *mode)?;
            #[cfg(not(unix))]
            gripsack_fs::atomic_write(dest_dir, dest_name, &bytes)?;
            Ok(())
        }
        store::Prior::Symlink { target } => {
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
/// Is the destination still exactly what this manifest entry
/// deployed? (Merge blocks are checked by block hash at the call
/// sites — a foreign file is never "intact" as a whole.)
pub fn intact_deployed(dest: &Path, entry: &store::DeployedEntry, store_path: &Path) -> bool {
    if entry.preserved_drift {
        return false;
    }
    match entry.mode {
        Ownership::Owned => std::fs::read_link(dest)
            .map(|t| t == store_path.join(&entry.from))
            .unwrap_or(false),
        Ownership::Merge => false, // merge never carries a prior
        Ownership::TrackedCopy | Ownership::Template => {
            matches!(super::observe_readonly(dest),
                Ok(Some(super::Observation::File { bytes, mode })) if entry.matches_file(&bytes, mode))
        }
    }
}

/// The intended post-prune identity of a destination (0026 §6):
/// the restored prior's identity when a prior exists, REMOVED
/// otherwise. Known BEFORE the mutation, from the prior blob —
/// never observed afterward.
pub fn prune_intent(
    entry: &store::DeployedEntry,
    home: &Path,
) -> std::io::Result<store::journal::Intended> {
    use store::journal::{Intended, ObjectIdentity};
    match &entry.prior {
        // restoring the prior: the intended identity is the prior's
        // own — mode-aware for files (0031), verbatim for links
        Some(store::Prior::File { hash, mode }) => {
            let bytes = std::fs::read(store::prior_blob_path(home, hash))?;
            Ok(Intended::Object(ObjectIdentity::File(
                store::canonical_bytes_identity(&bytes, *mode),
            )))
        }
        Some(store::Prior::Symlink { target }) => {
            Ok(Intended::Object(ObjectIdentity::Link(target.clone())))
        }
        None => Ok(Intended::Removed),
    }
}
