//! Removal of deployed destinations (prune-on-undeclare, 0006) with
//! exact-target guards (0030 §15).

use super::restore::restore_prior;
use gripsack_ir::Ownership;
use gripsack_store as store;
use std::path::Path;

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
    store_path: &Path,
) -> std::io::Result<bool> {
    if entry.preserved_drift {
        return Ok(false);
    }
    match entry.mode {
        Ownership::Owned => {
            // removal authority is the EXACT expected target (0030
            // §15): a user-repointed link to another gripsack object
            // is drift, never deletable
            let ours = dest_dir
                .read_link_contents(dest_name)
                .map(|t| t == store_path.join(&entry.from))
                .unwrap_or(false);
            if !ours {
                return Ok(false);
            }
            remove_if_present(dest_dir, dest_name)?;
            Ok(true)
        }
        Ownership::Merge => {
            let observation = super::observe(dest_dir, dest_name)?;
            let Some(super::Observation::File { bytes, mode }) = observation else {
                return Ok(observation.is_none());
            };
            let existing = String::from_utf8(bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let blocks = crate::managed_blocks::ManagedBlockSet::parse(&existing, module)
                .map_err(std::io::Error::other)?;
            if blocks.intact(entry, mode) {
                let new = blocks.remove().expect("intact implies a block");
                if new.is_empty() {
                    remove_if_present(dest_dir, dest_name)?;
                } else {
                    gripsack_fs::atomic_write(dest_dir, dest_name, new.as_bytes())?;
                }
                Ok(true)
            } else {
                Ok(false)
            }
        }
        Ownership::TrackedCopy | Ownership::Template => {
            let observation = super::observe(dest_dir, dest_name)?;
            let Some(super::Observation::File { bytes, mode }) = observation else {
                return Ok(observation.is_none());
            };
            if !entry.matches_file(&bytes, mode) {
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
    match gripsack_fs::remove_file(dest_dir, dest_name) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Record what a destination is before a take-over absorbs it (0015
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
    store_path: &Path,
) -> std::io::Result<bool> {
    if let Some(prior) = &entry.prior {
        restore_prior(dest_dir, dest_name, prior, home)?;
        return Ok(true);
    }
    remove_entry_deployed(dest_dir, dest_name, entry, module, store_path)
}
