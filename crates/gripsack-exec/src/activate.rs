//! Post-commit activation with stable intent identity and durable outcomes.
//! Known failure remains a warning/no-retry result. Interrupted Started records
//! are ambiguous and replay under the same intent ID with a new attempt number.
mod effect;
mod output;

use crate::progress::{
    Reporter, Verification, VerificationKind, VerificationLabel, VerificationLabels, process_status,
};
use crate::report::{ReportKind, StepReport};
use gripsack_store as store;
use gripsack_store::activation::{ActivationAction as Action, EffectiveIntent, IntentFailure};
use std::{io, path::Path, time::Instant};

/// Rollback uses retained actions, never today's repository source. Removal
/// declarations are added separately only for modules the rollback removes.
pub(crate) fn collect_from_manifest(
    generation: &store::Generation,
) -> Vec<store::activation::PendingIntent> {
    let mut declarations = Vec::new();
    for (name, state) in &generation.deployments {
        for record in &state.intents {
            if record.trigger != gripsack_store::ActivationTrigger::OnRemove {
                declarations.push(store::activation::PendingIntent {
                    output: name.clone(),
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
    reporter: Reporter<'_>,
    labels: &VerificationLabels,
) -> io::Result<Vec<StepReport>> {
    let Some(mut ready) = batch.authorize(home, home_path)? else {
        return Ok(Vec::new());
    };
    let environment = gripsack_process::OperatorEnvironment::capture();
    let directory = std::env::temp_dir();
    let mut reports = Vec::new();
    while let Some((permit, intent)) = ready.next_attempt()? {
        crate::util::crash_hook("hook-after-start");
        let label = label(intent, labels);
        if let Some(label) = &label {
            reporter.started(
                &label.owner,
                &format!("running {} {}", label.kind.describe(), label.name),
            );
        }
        let started = Instant::now();
        let result = effect::run(intent, &permit, &environment, &directory, home_path);
        let duration = started.elapsed();
        crate::util::crash_hook("hook-after-effect");
        let outcome = if result.failure.is_some() {
            gripsack_policy::activation::Outcome::Failed
        } else {
            gripsack_policy::activation::Outcome::Succeeded
        };
        let detail = result.failure.as_ref().map(|failure| {
            let status = match failure {
                IntentFailure::Process => result
                    .processes
                    .last()
                    .map(process_status)
                    .unwrap_or_else(|| "failed".into()),
                IntentFailure::Admission { stage, .. } => {
                    format!("not admitted at {stage:?}").to_lowercase()
                }
            };
            format!(
                "{status}; intent {} attempt {} is recorded, no automatic retry",
                permit.intent_id(),
                permit.attempt().value()
            )
        });
        ready.finish(permit, outcome, result.processes, result.failure)?;
        crate::util::crash_hook("hook-after-receipt");
        let rendered = match label {
            Some(label) => {
                let (stdout, stderr, truncated) = result.output.parts();
                reporter.verified(&Verification {
                    label,
                    passed: detail.is_none(),
                    duration,
                    detail,
                    stdout: stdout.to_vec(),
                    stderr: stderr.to_vec(),
                    truncated,
                })
            }
            None => {
                reporter.step(&result.report);
                reporter.is_live()
            }
        };
        reports.push(result.report);
        if !rendered && let Err(error) = result.output.emit() {
            tracing::warn!(
                "hook diagnostic output unavailable ({error}); the durable outcome is retained"
            );
        }
    }
    crate::util::crash_hook("hooks-before-cleanup");
    ready.archive(home)?;
    Ok(reports)
}

/// How an activation effect renders: hooks and adapters are verification
/// blocks; scheduler registration is a plain step.
fn label(intent: &EffectiveIntent, labels: &VerificationLabels) -> Option<VerificationLabel> {
    let owner = || intent.contributors()[0].output.clone();
    let adapter = |name: &str, owner: String, command: String| VerificationLabel {
        kind: VerificationKind::Adapter,
        name: name.to_owned(),
        owner,
        command,
    };
    Some(match intent.action() {
        Action::WorkspaceHook { context, .. } => {
            labels
                .get(context)
                .cloned()
                .unwrap_or_else(|| VerificationLabel {
                    kind: VerificationKind::Hook,
                    name: "workspace hook".into(),
                    owner: owner(),
                    command: "(retained command)".into(),
                })
        }
        Action::CustomShell { script } => adapter(
            "custom hook",
            owner(),
            format!(
                "sh -c '{}'",
                script
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("")
            ),
        ),
        Action::Fonts => adapter(
            "fontconfig cache refresh",
            "fonts".into(),
            "fc-cache -f".into(),
        ),
        Action::DesktopEntry => adapter(
            "desktop database refresh",
            "desktop-entry".into(),
            "update-desktop-database".into(),
        ),
        Action::Service { name, user } => adapter(
            &format!("service {name}"),
            owner(),
            format!(
                "systemctl {}enable --now {name}",
                if *user { "--user " } else { "" }
            ),
        ),
        Action::ScheduledJob { .. } => return None,
    })
}

/// Resume under the same typed lifecycle authority used by apply/rollback/GC.
/// An unfinished journal must be reconciled first; this never grants hook
/// authority merely because a caller knows a generation number.
pub fn resume_activation(
    session: &crate::LifecycleSession,
    reporter: Reporter<'_>,
) -> io::Result<Vec<StepReport>> {
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
    let mut reports = run(
        batch,
        &home,
        home_path,
        reporter,
        &VerificationLabels::new(),
    )?;
    if !reports.is_empty() {
        let resumed = StepReport {
            output: "*".into(),
            summary: "resumed interrupted activation; durable outcomes are retained".into(),
            kind: ReportKind::Warned,
        };
        reporter.step(&resumed);
        reports.insert(0, resumed);
    }
    Ok(reports)
}
