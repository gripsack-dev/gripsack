//! Restore and prior capture (0001 §3.5, 0015 §4): the ONE
//! deploy-restore path, shared by rollback — every mode gets its
//! correct semantics, never a naive byte copy (template re-renders
//! with the recorded vars; merge re-upserts only the block into the
//! foreign file).

use super::{dest_capability, read_foreign_text};
use gripsack_ir::Ownership;
use gripsack_store as store;
use std::path::Path;

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
    /// Tracked copy / rendered template / merge-upserted whole file —
    /// landed with EXACTLY this mode (0031: the manifest's recorded
    /// mode; the live mode when unrecorded; 0644 when absent).
    Bytes { bytes: Vec<u8>, mode: u32 },
}

/// The destination's current full permission mode, if it exists
/// (0026 §7's preserve rule, made explicit at plan time so the
/// journaled intent can name the landed identity exactly).
fn live_mode(dest: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(dest).ok().map(|m| m.mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = dest;
        None
    }
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
                Ok(new) => {
                    // the file is foreign — the restore keeps ITS
                    // mode, and the intent says so exactly (0031)
                    let mode = live_mode(dest).unwrap_or(0o644);
                    Ok(Some(RestorePlan {
                        intent: store::canonical_bytes_identity(new.as_bytes(), mode),
                        write: RestoreWrite::Bytes {
                            bytes: new.into_bytes(),
                            mode,
                        },
                    }))
                }
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
            // exact mode restoration (0031): the manifest's recorded
            // mode; the live mode on pre-0.27 manifests; 0644 fresh
            let mode = entry.file_mode.or_else(|| live_mode(dest)).unwrap_or(0o644);
            Ok(Some(RestorePlan {
                intent: store::canonical_bytes_identity(&rendered, mode),
                write: RestoreWrite::Bytes {
                    bytes: rendered,
                    mode,
                },
            }))
        }
        Ownership::TrackedCopy => {
            let bytes = std::fs::read(&source)?;
            let mode = entry.file_mode.or_else(|| live_mode(dest)).unwrap_or(0o644);
            Ok(Some(RestorePlan {
                intent: store::canonical_bytes_identity(&bytes, mode),
                write: RestoreWrite::Bytes { bytes, mode },
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
        RestoreWrite::Bytes { bytes, mode } => {
            gripsack_fs::atomic_write_with_mode(dest_dir, dest_name, bytes, *mode)
        }
    }
}

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
pub(crate) fn restore_prior(
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
