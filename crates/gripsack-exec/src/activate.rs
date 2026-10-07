//! Post-commit activation with stable intent identity and durable outcomes.
//! Known failure remains a warning/no-retry result. Interrupted Started records
//! are ambiguous and replay under the same intent ID with a new attempt number.
mod effect;
mod output;

use crate::report::{ReportKind, StepReport};
use gripsack_ir::step::StepAction;
use gripsack_store as store;
use std::{collections::BTreeMap, io, path::Path};

/// Collect declarations once; the store resolves effective cache coalescing
/// and order before publishing the immutable activation plan.
pub(crate) fn collect(
    order: &[String],
    steps_by_module: &BTreeMap<String, gripsack_ir::prepared::PreparedModule>,
) -> Vec<store::activation::PendingIntent> {
    let mut declarations = Vec::new();
    for name in order {
        for step in steps_by_module[name.as_str()].steps() {
            if let StepAction::Intent { action, trigger } = &step.action
                && *trigger != gripsack_ir::Trigger::OnRemove
            {
                declarations.push(store::activation::PendingIntent {
                    module: name.clone(),
                    action: action.as_ref().clone().into(),
                    trigger: *trigger,
                });
            }
        }
    }
    declarations
}

/// Rollback uses retained actions, never today's repository source. Removal
/// declarations are added separately only for modules the rollback removes.
pub(crate) fn collect_from_manifest(
    generation: &store::Generation,
) -> Vec<store::activation::PendingIntent> {
    let mut declarations = Vec::new();
    for (name, state) in &generation.modules {
        for record in &state.intents {
            if record.trigger != gripsack_ir::Trigger::OnRemove {
                declarations.push(store::activation::PendingIntent {
                    module: name.clone(),
                    action: record.action.clone(),
                    trigger: record.trigger,
                });
            }
        }
    }
    declarations
}

pub(crate) fn run(
    batch: store::activation::ActivationBatch,
    home: &gripsack_fs::Dir,
    home_path: &Path,
) -> io::Result<Vec<StepReport>> {
    let Some(mut ready) = batch.authorize(home, home_path)? else {
        return Ok(Vec::new());
    };
    let environment = gripsack_process::OperatorEnvironment::capture();
    let directory = std::env::temp_dir();
    let mut reports = Vec::new();
    while let Some((permit, intent)) = ready.next_attempt()? {
        crate::util::crash_hook("hook-after-start");
        let result = effect::run(intent, &permit, &environment, &directory, home_path);
        crate::util::crash_hook("hook-after-effect");
        let outcome = if result.failure.is_some() {
            gripsack_policy::activation::Outcome::Failed
        } else {
            gripsack_policy::activation::Outcome::Succeeded
        };
        ready.finish(permit, outcome, result.processes, result.failure)?;
        reports.push(result.report);
        crate::util::crash_hook("hook-after-receipt");
        if let Err(error) = result.output.emit() {
            tracing::warn!(
                "hook diagnostic output unavailable ({error}); the durable outcome is retained"
            );
        }
    }
    crate::util::crash_hook("hooks-before-cleanup");
    ready.archive(home)?;
    Ok(reports)
}

/// Resume under the same typed lifecycle authority used by apply/rollback/GC.
/// An unfinished journal must be reconciled first; this never grants hook
/// authority merely because a caller knows a generation number.
pub fn resume_activation(session: &crate::LifecycleSession) -> io::Result<Vec<StepReport>> {
    let home_path = session.home();
    let home = gripsack_fs::open(home_path)?;
    if store::journal::pending_recovery(&home)?.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "activation requires a reconciled transaction journal",
        ));
    }
    let Some(batch) = store::activation::load_pending(&home, home_path)? else {
        return Ok(Vec::new());
    };
    let mut reports = run(batch, &home, home_path)?;
    if !reports.is_empty() {
        reports.insert(
            0,
            StepReport {
                module: "*".into(),
                summary: "resumed interrupted activation; durable outcomes are retained".into(),
                kind: ReportKind::Warned,
            },
        );
    }
    Ok(reports)
}
