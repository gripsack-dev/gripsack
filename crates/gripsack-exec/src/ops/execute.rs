//! The VM (0034): each op executes under the journal — precondition
//! from the plan-time observation, the kind's write, postcondition on
//! the recorded intent.

use super::*;
use crate::ctx::ExecError;
use crate::report::ReportKind;

/// The destination-capability pair an op pins (0027 §5, 0030 §P0-1).
fn dest_capability(dest: &Path) -> std::io::Result<(gripsack_fs::Dir, PathBuf)> {
    crate::deploy::dest_capability(dest)
}

/// Execute one op under the journal (0034): the precondition is the
/// op's plan-time observation, re-validated at the mutation; the
/// postcondition is the recorded intent. Returns the report row and a
/// take-over's captured prior (the entry assembly merges it).
pub(crate) fn execute_op(
    home_dir: &gripsack_fs::Dir,
    home: &Path,
    op: &Op,
) -> Result<(OpReport, Option<store::Prior>), ExecError> {
    let fail = |detail: String| ExecError::Step {
        module: op.module.clone(),
        step: "deploy".into(),
        detail,
    };
    match &op.kind {
        // marker ops never execute — plan-time only
        OpKind::RunEffect | OpKind::Deferred => {
            unreachable!("marker ops are preview-only; the scheduler runs their steps directly")
        }
        OpKind::Satisfied => Ok((
            OpReport {
                // the mode's voice (0.21.1): a satisfied merge block
                // is not an unchanged file
                summary: match op.mode {
                    gripsack_ir::Ownership::Merge => format!("{} block unchanged", op.declared_to),
                    _ => format!("{} unchanged", op.declared_to),
                },
                kind: ReportKind::Satisfied,
            },
            None,
        )),
        OpKind::Preserved => {
            let note = match op.authority {
                Some(Authority::Foreign) => format!(
                    "{} exists, not deployed by gripsack — kept (needs --take-over)",
                    op.declared_to
                ),
                _ => format!("{} drifted — kept", op.declared_to),
            };
            tracing::warn!("{}", note);
            Ok((
                OpReport {
                    summary: note,
                    kind: ReportKind::Warned,
                },
                None,
            ))
        }
        OpKind::Link { target } => {
            let (dest_dir, dest_name) = dest_capability(&op.dest)
                .map_err(|e| fail(format!("cannot open {} parent: {e}", op.declared_to)))?;
            // 0015 §4: a genuine take-over records what was there first
            let prior = if op.authority == Some(Authority::TakeOver) {
                crate::deploy::restore::capture_prior(&dest_dir, &dest_name, home_dir)?
            } else {
                None
            };
            crate::deploy::journaled(
                home_dir,
                &dest_dir,
                &dest_name,
                &op.dest,
                op.intended.clone(),
                op.observed.clone(),
                || gripsack_fs::symlink_replace(&dest_dir, &dest_name, target),
            )?;
            Ok((
                OpReport {
                    summary: format!("linked {} → {}", op.module, op.declared_to),
                    kind: ReportKind::Installed,
                },
                prior,
            ))
        }
        OpKind::Write { content, mode } => {
            let bytes = match content {
                ContentSource::Bytes(b) => b.clone(),
                // deferred identities pin at apply — execution always
                // has bytes (the planner runs post-publish)
                ContentSource::DeferredFetch => {
                    return Err(fail(format!(
                        "{}: content not staged (deferred fetch reached execution)",
                        op.declared_to
                    )));
                }
            };
            let (dest_dir, dest_name) = dest_capability(&op.dest)
                .map_err(|e| fail(format!("cannot open {} parent: {e}", op.declared_to)))?;
            let prior = if op.authority == Some(Authority::TakeOver) {
                crate::deploy::restore::capture_prior(&dest_dir, &dest_name, home_dir)?
            } else {
                None
            };
            crate::deploy::journaled(
                home_dir,
                &dest_dir,
                &dest_name,
                &op.dest,
                op.intended.clone(),
                op.observed.clone(),
                || gripsack_fs::atomic_write_with_mode(&dest_dir, &dest_name, &bytes, *mode),
            )?;
            let verb = match op.authority {
                Some(Authority::Update) => "updated",
                Some(Authority::TakeOver) => "took over",
                _ => "copied",
            };
            Ok((
                OpReport {
                    summary: format!("{} {} → {}", verb, op.module, op.declared_to),
                    kind: ReportKind::Configured,
                },
                prior,
            ))
        }
        OpKind::MergeUpsert {
            payload,
            marker,
            existing,
            mode,
        } => {
            let (dest_dir, dest_name) = dest_capability(&op.dest)
                .map_err(|e| fail(format!("cannot open {} parent: {e}", op.declared_to)))?;
            crate::deploy::journaled(
                home_dir,
                &dest_dir,
                &dest_name,
                &op.dest,
                op.intended.clone(),
                op.observed.clone(),
                || {
                    // re-derive from the LATEST foreign content (0029
                    // §3): an outside-block write lands in the output
                    // or the precondition aborts — never silently lost
                    let latest = match dest_dir.read_to_string(&dest_name) {
                        Ok(t) => t,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                        Err(e) => return Err(e),
                    };
                    let new = crate::template::upsert_block(
                        &latest,
                        &op.module,
                        &op.dest,
                        marker.as_deref(),
                        &String::from_utf8_lossy(payload),
                        *mode,
                    )
                    .map_err(std::io::Error::other)?;
                    gripsack_fs::atomic_write_with_mode(
                        &dest_dir,
                        &dest_name,
                        new.as_bytes(),
                        *mode,
                    )
                },
            )?;
            // the report names what the plan saw (0.21.1 review):
            // regenerated hand-edits, stripped duplicates
            let note = super::plan::merge_notes_pub(&op.module, payload, existing);
            Ok((
                OpReport {
                    summary: format!("merged {} → {}{}", op.module, op.declared_to, note),
                    kind: ReportKind::Configured,
                },
                None,
            ))
        }
        OpKind::Remove => {
            let (entry, store_path) = op
                .removing
                .clone()
                .expect("a Remove op carries its manifest entry");
            let (dest_dir, dest_name) = dest_capability(&op.dest)
                .map_err(|e| fail(format!("cannot open {} parent: {e}", op.declared_to)))?;
            let entry_cloned = entry.clone();
            let home_path = home.to_path_buf();
            let module_name = op.module.clone();
            crate::deploy::journaled(
                home_dir,
                &dest_dir,
                &dest_name,
                &op.dest,
                op.intended.clone(),
                op.observed.clone(),
                || {
                    crate::deploy::remove_or_restore_prior(
                        &dest_dir,
                        &dest_name,
                        &entry_cloned,
                        &module_name,
                        &home_path,
                        &store_path,
                    )
                    .map(|_| ())
                },
            )?;
            Ok((
                OpReport {
                    summary: format!("removed {}", op.declared_to),
                    kind: ReportKind::Configured,
                },
                None,
            ))
        }
    }
}
