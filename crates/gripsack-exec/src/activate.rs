//! Post-commit activation with stable intent identity and durable outcomes,
//! decided by the kernel's activation machine (plan/0056 F2 step 5, P4).
//! Known failure remains a warning/no-retry result. Interrupted Started records
//! are ambiguous and replay under the same intent ID with a new attempt number.
mod effect;
mod output;

use crate::progress::{
    Reporter, Verification, VerificationKind, VerificationLabel, VerificationLabels, process_status,
};
use crate::report::{StepReport, Tone};
use gripsack_host::store;
use gripsack_host::store::activation::{
    ActivationAction as Action, EffectiveIntent, IntentFailure,
};
use gripsack_kernel::activation::{
    ActivationState, Command, Event, HookResult, IntentState, Refusal, activation_step,
};
use gripsack_kernel::recover::Failure;
use std::{io, path::Path, time::Instant};

/// Rollback uses retained actions, never today's repository source. Removal
/// declarations are added separately only for modules the rollback removes.
pub(crate) fn collect_from_manifest(
    generation: &store::Generation,
) -> Vec<store::activation::PendingIntent> {
    let mut declarations = Vec::new();
    for (name, state) in &generation.deployments {
        for record in &state.intents {
            if record.trigger != gripsack_host::store::ActivationTrigger::OnRemove {
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

/// The driver of the kernel's activation machine (`gripsack_kernel::activation`):
/// it executes exactly the commanded step, re-checks `current` against a
/// launch's `expect` immediately before the hook runs, and reports.
pub(crate) fn run(
    batch: store::activation::ActivationBatch,
    home: &gripsack_host::fs::Dir,
    home_path: &Path,
    reporter: Reporter<'_>,
    labels: &VerificationLabels,
) -> io::Result<Vec<StepReport>> {
    let mut batch = batch;
    let mut machine = ActivationState::new();
    let mut event = Event::Start;
    let mut environment = None;
    let directory = std::env::temp_dir();
    let mut reports = Vec::new();
    // The returned attempt whose terminal record the kernel commands next.
    let mut returned: Option<Returned> = None;
    loop {
        let (next, command) = activation_step(machine, event);
        machine = next;
        event = match command {
            Command::ReadBatch => Event::BatchRead(batch.facts()),
            Command::ReadCurrent => Event::CurrentRead(current(home, home_path)?),
            Command::Authorize(_) => {
                batch.authorize(home, home_path)?;
                Event::Done
            }
            Command::ReadIntent(index) => Event::IntentRead(batch.record(index)?),
            Command::Persist { intent, state } => {
                let settles = matches!(
                    state,
                    IntentState::Succeeded { .. } | IntentState::Failed { .. }
                );
                match (settles, returned.take()) {
                    (true, Some(done)) => {
                        batch.persist(intent, state, done.effect.processes, done.effect.failure)?;
                        crate::util::crash_hook("hook-after-receipt");
                        reports.push(render(
                            reporter,
                            done.label,
                            done.duration,
                            done.detail,
                            done.effect.report,
                            done.effect.output,
                        ));
                    }
                    (false, None) => {
                        batch.persist(intent, state, Vec::new(), None)?;
                        if matches!(state, IntentState::Started { .. }) {
                            crate::util::crash_hook("hook-after-start");
                        }
                    }
                    _ => {
                        return Err(invalid(
                            "activation outcome does not match its returned attempt",
                        ));
                    }
                }
                Event::Done
            }
            Command::RunHook {
                intent,
                attempt,
                expect,
            } => {
                if current(home, home_path)? != Some(expect) {
                    Event::Failed(Failure::Drift)
                } else {
                    let (permit, intent) = batch.launch(intent, attempt)?;
                    let label = label(intent, labels);
                    if let Some(label) = &label {
                        reporter.started(
                            &label.owner,
                            &format!("running {} {}", label.kind.describe(), label.name),
                        );
                    }
                    let environment = environment
                        .get_or_insert_with(gripsack_host::process::OperatorEnvironment::capture);
                    let started = Instant::now();
                    let effect = effect::run(intent, &permit, environment, &directory, home_path);
                    let duration = started.elapsed();
                    crate::util::crash_hook("hook-after-effect");
                    let result = if effect.failure.is_some() {
                        HookResult::Failed
                    } else {
                        HookResult::Succeeded
                    };
                    let detail = effect.failure.as_ref().map(|failure| {
                        let status = match failure {
                            IntentFailure::Process => effect
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
                    returned = Some(Returned {
                        label,
                        duration,
                        detail,
                        effect,
                    });
                    Event::Returned(result)
                }
            }
            Command::Archive => {
                crate::util::crash_hook("hooks-before-cleanup");
                batch.archive()?;
                Event::Done
            }
            Command::Clear => {
                batch.clear(home)?;
                Event::Done
            }
            Command::Finish(_) => return Ok(reports),
            Command::Abort(refusal) => return Err(refused(refusal)),
        };
    }
}

/// A hook that returned, held until the kernel commands its terminal record.
struct Returned {
    label: Option<VerificationLabel>,
    duration: std::time::Duration,
    detail: Option<String>,
    effect: effect::EffectOutcome,
}

fn current(
    home: &gripsack_host::fs::Dir,
    home_path: &Path,
) -> io::Result<Option<gripsack_kernel::selection::SelectionIdentity>> {
    store::generations::current_selection_in(home_path, home)
}

/// Render a settled attempt: hooks and adapters as verification blocks,
/// everything else as a step.
fn render(
    reporter: Reporter<'_>,
    label: Option<VerificationLabel>,
    duration: std::time::Duration,
    detail: Option<String>,
    report: StepReport,
    output: output::CapturedOutput,
) -> StepReport {
    let rendered = match label {
        Some(label) => {
            let (stdout, stderr, truncated) = output.parts();
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
            reporter.step(&report);
            reporter.is_live()
        }
    };
    if !rendered && let Err(error) = output.emit() {
        tracing::warn!(
            "hook diagnostic output unavailable ({error}); the durable outcome is retained"
        );
    }
    report
}

fn refused(refusal: Refusal) -> io::Error {
    invalid(match refusal {
        Refusal::Exhausted { .. } => "activation attempt counter exhausted; evidence retained",
        Refusal::Drifted { .. } => {
            "the selected generation changed before a hook launched; evidence retained"
        }
        Refusal::Malformed => "activation intent count overflowed; evidence retained",
        Refusal::Failed | Refusal::Protocol => {
            "activation stopped on an unexpected step; evidence retained"
        }
    })
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
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
    let home = gripsack_host::fs::open(home_path)?;
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
            tone: Tone::Warned,
        };
        reporter.step(&resumed);
        reports.insert(0, resumed);
    }
    Ok(reports)
}
