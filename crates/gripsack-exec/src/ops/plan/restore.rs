//! Rollback prepares retained inputs for the same entry planner as apply.
use super::*;

/// A restore op for rollback (0034): the desired state is the TARGET
/// generation's manifest record, planned by the same per-destination
/// function apply uses. None when no safe restore exists (the caller
/// surfaces a Skipped note — 0030 §H8).
pub(crate) fn plan_restore_op(
    module: &str,
    entry: &store::DeployedEntry,
    store_path: &Path,
    prev: Option<&store::DeployedEntry>,
    home: &Path,
) -> Result<Option<Op>, ExecError> {
    let fail = |detail: String| ExecError::Step {
        module: module.to_string(),
        step: "rollback".into(),
        detail,
    };
    // the planner speaks IR entries; the manifest record synthesizes
    // one (the marker is deploy-time only — merge restore upserts
    // with the default)
    let ir_entry = Entry {
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
        module,
        entry: &ir_entry,
        dest,
        home,
        observed,
        prev,
        take_over: false, // rollback never absorbs
    };
    let mut op = match entry.ownership.policy() {
        Ownership::Owned => {
            if !source.exists() {
                return Ok(None);
            }
            let already = std::fs::read_link(&view.dest)
                .map(|t| t == source)
                .unwrap_or(false);
            plan_entry_op(
                &view,
                ModeInput::Link {
                    source: &source,
                    content_hash: store::canonical_file_hash(&source)
                        .map_err(|e| fail(format!("{e}")))?
                        .into(),
                    already,
                },
            )?
        }
        Ownership::TrackedCopy | Ownership::Template => {
            let bytes = match std::fs::read(&source) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(fail(format!("{e}"))),
            };
            let rendered;
            let content: &[u8] = match entry.ownership.policy() {
                Ownership::Template => {
                    rendered = crate::template::render_template(
                        &bytes,
                        &entry.vars,
                        &entry.from.to_string_lossy(),
                    )
                    .map_err(|e| fail(format!("{e}")))?;
                    &rendered
                }
                _ => &bytes,
            };
            // exact mode restoration (0031): the recorded landed mode;
            // an unrecorded one follows the payload's exec bit
            #[cfg(unix)]
            let payload_exec = {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(&source)
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
            };
            #[cfg(not(unix))]
            let payload_exec = false;
            let intent_mode = entry
                .file_mode
                .unwrap_or(if payload_exec { 0o755 } else { 0o644 });
            plan_entry_op(
                &view,
                ModeInput::Write {
                    content,
                    permissions: WritePermissions::Exact(intent_mode),
                },
            )?
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
