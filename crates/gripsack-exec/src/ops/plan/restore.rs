//! Rollback restores exactly what the target generation recorded
//! (plan/0056 K2): retained store bytes whose identity IS the record, or
//! nothing. Re-rendering an older template receipt and choosing a mode for a
//! receipt that recorded none are allowed only because the result is checked
//! against the recorded identity before anything is installed.
use super::*;

/// The retained bytes that reproduce one whole-object record exactly.
pub(crate) enum Retained {
    /// Content and mode whose identity is the record (a tracked copy, a
    /// workspace template's rendered output, an older template receipt
    /// re-rendered with its recorded variables).
    File { content: Vec<u8>, mode: u32 },
    /// The owned link's store payload, whose content hash is the record.
    Link {
        source: PathBuf,
        content_hash: store::hash::ManifestHash,
    },
    /// No retained bytes reproduce the record.
    Missing,
}

/// The retained bytes for a whole-object entry, verified against its record.
/// A merge block is not a whole-object unit (K5) and has none.
pub(crate) fn retained(
    output: &str,
    entry: &store::DeployedEntry,
    store_path: &Path,
) -> Result<Retained, ExecError> {
    let fail = |detail: String| ExecError::Step {
        output: output.to_string(),
        step: "rollback".into(),
        detail,
    };
    let source = store_path.join(&entry.from);
    match entry.ownership.policy() {
        Ownership::Owned => {
            if std::fs::symlink_metadata(&source).is_err() {
                return Ok(Retained::Missing);
            }
            let hash = store::canonical_file_hash(&source).map_err(|e| fail(format!("{e}")))?;
            if hash.as_str() != entry.hash.as_str() {
                return Ok(Retained::Missing);
            }
            Ok(Retained::Link {
                source,
                content_hash: hash.into(),
            })
        }
        Ownership::TrackedCopy | Ownership::Template => {
            let bytes = match std::fs::read(&source) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Retained::Missing);
                }
                Err(e) => return Err(fail(format!("{e}"))),
            };
            let content = match entry.ownership.policy() {
                // an older receipt (0.16.1 vars-in-manifest): re-render,
                // then let the record decide whether that is what landed
                Ownership::Template => match crate::template::render_template(
                    &bytes,
                    &entry.vars,
                    &entry.from.to_string_lossy(),
                ) {
                    Ok(rendered) => rendered,
                    Err(_) => return Ok(Retained::Missing),
                },
                _ => bytes,
            };
            // the recorded landed mode (0031); a receipt that recorded none
            // is tried at the payload's executable default, and the record
            // decides whether that guess is the identity that landed
            let mode = entry.file_mode.unwrap_or_else(|| {
                if payload_executable(&source) {
                    0o755
                } else {
                    0o644
                }
            });
            if entry.matches_file(&content, mode) {
                Ok(Retained::File { content, mode })
            } else {
                Ok(Retained::Missing)
            }
        }
        Ownership::Merge => Ok(Retained::Missing),
    }
}

fn payload_executable(source: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(source).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = source;
        false
    }
}

/// A restore op for rollback (0034): the desired state is the TARGET
/// generation's manifest record, planned by the same per-destination
/// function apply uses. None when no exact restore exists — the caller
/// refuses the rollback before it flips.
pub(crate) fn plan_restore_op(
    output: &str,
    entry: &store::DeployedEntry,
    store_path: &Path,
    prev: Option<PreviousRecord<'_>>,
    home: &Path,
) -> Result<Option<Op>, ExecError> {
    let fail = |detail: String| ExecError::Step {
        output: output.to_string(),
        step: "rollback".into(),
        detail,
    };
    // Restore uses the same deployment input as apply, with the retained
    // ownership and variables. Managed-block restoration supplies its marker.
    let deployment = DeploymentEntry {
        from: entry.from.to_string_lossy().into_owned(),
        to: entry.to.clone(),
        mode: entry.ownership.policy().clone(),
        vars: entry.vars.clone(),
        marker: None,
        span: None,
    };
    let dest = entry.key();
    let observed = crate::deploy::observe_readonly(&dest)
        .map_err(|error| fail(format!("cannot inspect {}: {error}", entry.to)))?;
    let source = store_path.join(&entry.from);
    let view = DestView {
        output,
        entry: &deployment,
        dest,
        home,
        observed,
        prev,
        consent: take_over_consent(false), // rollback never absorbs
    };
    let mut op = match entry.ownership.policy() {
        Ownership::Owned | Ownership::TrackedCopy | Ownership::Template => {
            match retained(output, entry, store_path)? {
                Retained::Missing => return Ok(None),
                Retained::Link {
                    source,
                    content_hash,
                } => plan_entry_op(
                    &view,
                    ModeInput::Link {
                        source: &source,
                        content_hash,
                    },
                )?,
                Retained::File { content, mode } => plan_entry_op(
                    &view,
                    ModeInput::Write {
                        content: &content,
                        permissions: WritePermissions::Exact(mode),
                    },
                )?,
            }
        }
        Ownership::Merge => {
            let payload = match std::fs::read_to_string(&source) {
                Ok(t) => t,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(fail(format!("{e}"))),
            };
            match &view.observed {
                None => {}
                Some(crate::deploy::Observation::File { bytes, .. })
                    if std::str::from_utf8(bytes).is_ok() => {}
                _ => return Ok(None),
            }
            plan_entry_op(
                &view,
                ModeInput::Merge {
                    block_id: entry.ownership.block_id(),
                    payload: &payload,
                    permissions: entry
                        .file_mode
                        .map_or(WritePermissions::Preserve, WritePermissions::Exact),
                },
            )?
        }
    };
    if let Some(produced) = &mut op.produces
        && !produced.preserved_drift
    {
        produced.source_executable = entry.source_executable;
    }
    Ok(Some(op))
}
