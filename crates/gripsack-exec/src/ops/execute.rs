//! The VM (0034): each op executes under the journal — precondition
//! from the plan-time observation, the kind's write, postcondition on
//! the recorded intent.

use super::*;
use crate::ctx::ExecError;
use crate::report::Tone;
use gripsack_host::storage::Permissions;
use store::journal::Install;

/// The destination-capability pair an op pins (0027 §5, 0030 §P0-1).
fn dest_capability(
    run: &store::journal::JournalRun<'_>,
    dest: &Path,
) -> std::io::Result<(gripsack_fs::Dir, PathBuf)> {
    match store::journal::directory::destination_capability(run, dest)? {
        Some(bound) => Ok(bound),
        None => crate::deploy::dest_capability(dest),
    }
}

/// What executing one op did.
pub(crate) struct Executed {
    /// The report row.
    pub report: OpReport,
    /// A take-over's captured prior (the entry assembly merges it).
    pub prior: Option<store::Prior>,
    /// What landed, for an op that mutated (the leaf protocol accepted it as
    /// the op's intended object; the apply machine judges it again).
    pub landed: Option<gripsack_kernel::recover::Live>,
}

/// Execute one op under the journal (0034): the precondition is the
/// op's plan-time observation, re-validated at the mutation; the
/// postcondition is the recorded intent.
pub(crate) fn execute_op(
    run: &store::journal::JournalRun<'_>,
    op: ExecutableOp<'_>,
) -> Result<Executed, ExecError> {
    let op = op.0;
    let home_dir = run.home();
    let fail = |detail: String| ExecError::Step {
        output: op.output().to_string(),
        step: "deploy".into(),
        detail,
    };
    match op.kind() {
        // the ExecutableOp conversion already refused markers
        // (0046) — this arm is a classified error, never a panic
        OpKind::Deferred => Err(fail("a preview-only marker op reached the executor".into())),
        OpKind::Satisfied => Ok(Executed {
            report: OpReport {
                // the mode's voice (0.21.1): a satisfied merge block
                // is not an unchanged file
                summary: match op.mode {
                    gripsack_store::Ownership::Merge => {
                        format!("{} block unchanged", op.declared_to())
                    }
                    _ => format!("{} unchanged", op.declared_to()),
                },
                tone: Tone::Unchanged,
            },
            prior: None,
            landed: None,
        }),
        OpKind::Preserved => {
            let note = match op.authority() {
                Some(Authority::Foreign) => format!(
                    "{} exists, not deployed by gripsack — kept (needs --take-over)",
                    op.declared_to()
                ),
                Some(Authority::Drifted) => format!(
                    "{} changed since gripsack deployed it — kept (needs --take-over)",
                    op.declared_to()
                ),
                _ => format!("{} drifted — kept", op.declared_to()),
            };
            tracing::warn!("{}", note);
            Ok(Executed {
                report: OpReport {
                    summary: note,
                    tone: Tone::Warned,
                },
                prior: None,
                landed: None,
            })
        }
        OpKind::Link { target } => {
            let (dest_dir, dest_name) = dest_capability(run, op.dest())
                .map_err(|e| fail(format!("cannot open {} parent: {e}", op.declared_to())))?;
            // 0015 §4: a genuine take-over records what was there first
            let prior = if op.authority() == Some(Authority::TakeOver) {
                crate::deploy::restore::capture_prior(&dest_dir, &dest_name, home_dir)?
            } else {
                None
            };
            let landed = store::journal::mutate(
                run,
                dest_dir,
                dest_name,
                op.dest(),
                op.observed(),
                op.intended(),
                Install::Link {
                    target: target.clone(),
                },
            )?;
            Ok(Executed {
                report: OpReport {
                    summary: format!("linked {} → {}", op.output(), op.declared_to()),
                    tone: Tone::Done,
                },
                prior,
                landed: Some(landed),
            })
        }
        OpKind::Write { content, mode } => {
            let bytes = match content {
                ContentSource::Bytes(bytes) => bytes.as_slice(),
                // deferred identities pin at apply — execution always
                // has bytes (the planner runs post-publish)
                ContentSource::DeferredFetch => {
                    return Err(fail(format!(
                        "{}: content not staged (deferred fetch reached execution)",
                        op.declared_to()
                    )));
                }
            };
            let (dest_dir, dest_name) = dest_capability(run, op.dest())
                .map_err(|e| fail(format!("cannot open {} parent: {e}", op.declared_to())))?;
            let prior = if op.authority() == Some(Authority::TakeOver) {
                crate::deploy::restore::capture_prior(&dest_dir, &dest_name, home_dir)?
            } else {
                None
            };
            let landed = store::journal::mutate(
                run,
                dest_dir,
                dest_name,
                op.dest(),
                op.observed(),
                op.intended(),
                Install::File {
                    bytes: bytes.to_vec(),
                    mode: Permissions::new(*mode),
                },
            )?;
            let verb = match op.authority() {
                Some(Authority::Update) => "updated",
                Some(Authority::TakeOver) => "took over",
                _ => "copied",
            };
            Ok(Executed {
                report: OpReport {
                    summary: format!("{} {} → {}", verb, op.output(), op.declared_to()),
                    tone: Tone::Done,
                },
                prior,
                landed: Some(landed),
            })
        }
        OpKind::MergeUpsert {
            block_id,
            payload,
            marker,
            mode,
        } => {
            let owner = block_id
                .as_ref()
                .map_or(op.output(), store::ManagedBlockId::as_str);
            let (dest_dir, dest_name) = dest_capability(run, op.dest())
                .map_err(|e| fail(format!("cannot open {} parent: {e}", op.declared_to())))?;
            // re-derive from the LATEST bytes, which the leaf protocol's
            // re-check pins to the plan's observation (0029 §3): an
            // outside-block write aborts the run — never silently lost
            let splice = |latest: &[u8], _: Option<Permissions>| {
                let latest = std::str::from_utf8(latest)
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
                let blocks = crate::managed_blocks::ManagedBlockSet::parse(latest, owner)
                    .map_err(std::io::Error::other)?;
                let new = blocks
                    .upsert(
                        owner,
                        op.dest(),
                        marker.as_deref(),
                        &String::from_utf8_lossy(payload),
                        *mode,
                    )
                    .map_err(std::io::Error::other)?;
                Ok(Some((new.into_bytes(), Permissions::new(*mode))))
            };
            let landed = store::journal::mutate(
                run,
                dest_dir,
                dest_name,
                op.dest(),
                op.observed(),
                op.intended(),
                Install::Splice(Box::new(splice)),
            )?;
            // the report names what the plan saw (0.21.1 review):
            // regenerated hand-edits, stripped duplicates
            let note = op.note.as_deref().unwrap_or("");
            Ok(Executed {
                report: OpReport {
                    summary: format!("merged {} → {}{}", op.output(), op.declared_to(), note),
                    tone: Tone::Done,
                },
                prior: None,
                landed: Some(landed),
            })
        }
        OpKind::Remove(target) => {
            // the variant CARRIES the removal authority (0046) — no
            // expect, no invalid state
            let (dest_dir, dest_name) = dest_capability(run, op.dest())
                .map_err(|e| fail(format!("cannot open {} parent: {e}", op.declared_to())))?;
            let landed = store::journal::mutate(
                run,
                dest_dir,
                dest_name,
                op.dest(),
                op.observed(),
                op.intended(),
                crate::deploy::removal(&target.entry, op.output(), home_dir)?,
            )?;
            Ok(Executed {
                report: OpReport {
                    summary: format!("removed {}", op.declared_to()),
                    tone: Tone::Done,
                },
                prior: None,
                landed: Some(landed),
            })
        }
    }
}
