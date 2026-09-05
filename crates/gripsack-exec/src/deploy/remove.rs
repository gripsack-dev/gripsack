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
            let existing = match dest_dir.read_to_string(dest_name) {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
                Err(e) => return Err(e),
            };
            match crate::template::extract_block(&existing, module) {
                Some(content)
                    if store::canonical_bytes_hash(content.as_bytes()).as_str() == entry.hash =>
                {
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
        Ownership::Template => {
            // bytes-only domain: only delete what we rendered
            let current = match dest_dir.read(dest_name) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
                Err(e) => return Err(e),
            };
            if store::canonical_bytes_hash(&current).as_str() != entry.hash {
                return Ok(false);
            }
            remove_if_present(dest_dir, dest_name)?;
            Ok(true)
        }
        Ownership::TrackedCopy => {
            // mode-aware domain (0031): only delete bytes+mode
            // identical to what we wrote — a chmodded copy is the
            // user's now
            let bytes = match dest_dir.read(dest_name) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
                Err(e) => return Err(e),
            };
            #[cfg(unix)]
            let mode = {
                use gripsack_fs::cap_std::fs::MetadataExt;
                dest_dir
                    .symlink_metadata(dest_name)
                    .map(|m| m.mode() & 0o7777)
                    .unwrap_or(0o644)
            };
            #[cfg(not(unix))]
            let mode = 0o644;
            if store::canonical_bytes_identity(&bytes, mode).as_str() != entry.hash {
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
