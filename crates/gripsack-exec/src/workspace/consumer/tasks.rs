//! One bounded manual/scheduled invocation over a fully admitted static graph.
//! Sequential topological execution deliberately bounds concurrency to one;
//! prerequisite success is invocation-local and includes every postcondition.
use super::{
    admit, clone_leases, command, consumer_limits, failure, gate, live_checkout, operational,
    task_graph::{self, TaskGraph},
    task_locks::{LockConflict, MutationClaims},
    task_report::{self, TaskNodeReport, TaskReport, TaskStatus},
};
use crate::workspace::{
    BuildOptions, realize,
    roots::{self, ProcessLease, RetentionSet},
};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_ir::{
    Ir,
    workspace_model::{WorkspaceOutput, WorkspaceStep},
};
use gripsack_process::{Invocation, ProcessLeases, ProcessRole};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

#[cfg(test)]
#[path = "task_capture_tests.rs"]
mod capture_tests;

#[derive(Debug, Clone, Copy)]
pub enum BusyPolicy {
    Fail,
    Skip,
}
#[derive(Debug, Clone, Copy)]
pub enum TaskStdin {
    Inherit,
    Closed,
}
#[derive(Debug, Clone, Copy)]
pub enum TaskOutputMode {
    Stream,
    Capture,
}
#[derive(Debug, Clone, Copy)]
pub struct TaskRunPolicy {
    pub on_busy: BusyPolicy,
    pub stdin: TaskStdin,
    pub output: TaskOutputMode,
}
impl Default for TaskRunPolicy {
    fn default() -> Self {
        Self {
            on_busy: BusyPolicy::Fail,
            stdin: TaskStdin::Inherit,
            output: TaskOutputMode::Stream,
        }
    }
}

enum ActionPhase {
    Step,
    Postcondition,
}
struct PreparedNode {
    commands: Vec<command::PreparedCommand>,
    checks: Vec<command::PreparedCommand>,
}

pub fn run_task(
    ir: &Ir,
    ctx: &Ctx,
    options: &BuildOptions<'_>,
    task: &str,
    policy: TaskRunPolicy,
) -> Result<TaskReport, ExecError> {
    task_graph::check_selection(ctx, task)?;
    let graph = TaskGraph::admit(ir, task)?;
    let mut report = TaskReport::new(task, graph.plan.revision.clone())?;
    report.persist(&ctx.home)?;
    task_report::persist_log(&ctx.home, &report, &[])?;
    // Common realization resolves the whole selected closure, including explicit
    // EnsureArtifact targets; neither task execution nor artifact hits are cached.
    let held = match realize::realize_held(ir, ctx, options, &ctx.only) {
        Ok(held) => held,
        Err(error) => {
            report.status = TaskStatus::Blocked;
            report.error = Some(error.to_string());
            report.persist(&ctx.home)?;
            return Ok(report);
        }
    };
    let closure: BTreeSet<_> = held
        .closures
        .iter()
        .flat_map(|(_, paths)| paths.iter().cloned())
        .collect();
    run_realized(
        ir,
        ctx,
        options,
        policy,
        graph,
        report,
        held.session,
        &held.realization,
        closure,
        None,
    )
}

/// Caller holds the same lifecycle fence used to select the active revision.
/// Registration of the process root happens before that fence is released.
#[allow(clippy::too_many_arguments)]
pub(in crate::workspace) fn run_captured(
    ir: &Ir,
    ctx: &Ctx,
    options: &BuildOptions<'_>,
    session: LifecycleSession,
    realization: &realize::Realization<'_>,
    closure: BTreeSet<PathBuf>,
    expected: &[Vec<gripsack_process::ProgramIdentity>],
    schedule: task_report::ScheduledInvocation,
) -> Result<TaskReport, ExecError> {
    let task = ctx
        .only
        .first()
        .ok_or_else(|| failure("captured task has no root"))?;
    task_graph::check_selection(ctx, task)?;
    let graph = TaskGraph::admit(ir, task)?;
    let mut report = TaskReport::new(task, graph.plan.revision.clone())?;
    report.schedule = Some(schedule);
    report.persist(&ctx.home)?;
    task_report::persist_log(&ctx.home, &report, &[])?;
    run_realized(
        ir,
        ctx,
        options,
        TaskRunPolicy {
            on_busy: BusyPolicy::Skip,
            stdin: TaskStdin::Closed,
            output: TaskOutputMode::Capture,
        },
        graph,
        report,
        session,
        realization,
        closure,
        Some(expected),
    )
}

#[allow(clippy::too_many_arguments)]
fn run_realized(
    ir: &Ir,
    ctx: &Ctx,
    options: &BuildOptions<'_>,
    policy: TaskRunPolicy,
    graph: TaskGraph<'_>,
    mut report: TaskReport,
    session: LifecycleSession,
    realization: &realize::Realization<'_>,
    closure: BTreeSet<PathBuf>,
    expected: Option<&[Vec<gripsack_process::ProgramIdentity>]>,
) -> Result<TaskReport, ExecError> {
    let services: BTreeSet<_> = graph
        .nodes
        .iter()
        .flat_map(|task| task.services.iter().cloned())
        .collect();
    if let Err(error) = crate::services::require_ready(
        ir,
        ctx,
        &services.into_iter().collect::<Vec<_>>(),
        options.deadline,
    ) {
        report.status = TaskStatus::Blocked;
        report.error = Some(error.to_string());
        report.persist(&ctx.home)?;
        return Ok(report);
    }
    report.execution_binding = Some(task_report::TaskExecutionBinding(
        gripsack_process::Sha256Digest::of(&serde_json::to_vec(&(
            &report.revision,
            ctx.repository.identity(),
            &closure,
        ))?),
    ));
    let retention = RetentionSet::admit(&session, closure)?;
    let lease = ProcessLease::register(&session, &retention)?;
    report.process_root = Some(lease.id().as_str().to_owned());
    report.root_retained = true;
    report.persist(&ctx.home)?;
    let staging = create_staging(ctx, lease.id().as_str())?;
    let prepared =
        prepare(&graph, ctx, options, realization, &staging, &ir.host).and_then(|nodes| {
            if let Some(expected) = expected
                && identities(&nodes) != expected
            {
                return Err(failure(
                    "captured task executable or interpreter identity changed",
                ));
            }
            Ok(nodes)
        });
    let nodes = match prepared {
        Ok(nodes) => nodes,
        Err(error) => {
            lease.release_unlaunched(&session)?;
            remove_staging(ctx, &staging)?;
            report.status = TaskStatus::Blocked;
            report.root_retained = false;
            report.error = Some(error.to_string());
            report.persist(&ctx.home)?;
            return Ok(report);
        }
    };
    let claims = match MutationClaims::acquire(
        &ctx.home,
        &graph.plan.mutation_locks,
        &report.invocation,
        lease.id().as_str(),
    )? {
        Ok(claims) => claims,
        Err(conflict) => {
            lease.release_unlaunched(&session)?;
            remove_staging(ctx, &staging)?;
            report.root_retained = false;
            report.status = match conflict {
                LockConflict::Unknown => TaskStatus::Indeterminate,
                LockConflict::Held => match policy.on_busy {
                    BusyPolicy::Fail => TaskStatus::Blocked,
                    BusyPolicy::Skip => TaskStatus::SkippedBusy,
                },
            };
            report.error = Some("mutation keys conflict with a live or unreconciled invocation; no commands launched".into());
            report.persist(&ctx.home)?;
            return Ok(report);
        }
    };
    let leases = ProcessLeases {
        worker: None,
        retention: Some(lease.duplicate_handle()?),
    };
    drop(session);
    let mut logs = Vec::new();
    let mut launched = false;
    let execution = execute(
        &graph,
        nodes,
        ctx,
        options,
        policy,
        &leases,
        &mut report,
        &mut logs,
        &mut launched,
    );
    drop(leases);
    if let Err(error) = execution {
        report.status = if launched {
            TaskStatus::Indeterminate
        } else {
            TaskStatus::Blocked
        };
        report.error = Some(error.to_string());
    }
    let unknown = report.status == TaskStatus::Indeterminate;
    if !unknown {
        // Every process outcome preceding this point confirmed cleanup. The
        // descriptor stayed held across the entire graph, not just its leader.
        let session = LifecycleSession::acquire(&ctx.home)?;
        if let Some(receipt) = report.last_receipt() {
            lease.release(&session, receipt)?;
        } else {
            lease.release_unlaunched(&session)?;
        }
        claims.release()?;
        remove_staging(ctx, &staging)?;
        report.root_retained = false;
    }
    task_report::persist_log(&ctx.home, &report, &logs)?;
    report.persist(&ctx.home)?;
    Ok(report)
}

pub(in crate::workspace) fn capture_programs(
    ir: &Ir,
    ctx: &Ctx,
    options: &BuildOptions<'_>,
    realization: &realize::Realization<'_>,
    staging: &Path,
) -> Result<Vec<Vec<gripsack_process::ProgramIdentity>>, ExecError> {
    let task = ctx
        .only
        .first()
        .ok_or_else(|| failure("captured task has no root"))?;
    let graph = TaskGraph::admit(ir, task)?;
    let nodes = prepare(&graph, ctx, options, realization, staging, &ir.host)?;
    Ok(identities(&nodes))
}

fn identities(nodes: &[PreparedNode]) -> Vec<Vec<gripsack_process::ProgramIdentity>> {
    nodes
        .iter()
        .map(|node| {
            node.commands
                .iter()
                .chain(&node.checks)
                .map(|command| command.program.identity())
                .collect()
        })
        .collect()
}

fn create_staging(ctx: &Ctx, id: &str) -> Result<PathBuf, ExecError> {
    gripsack_fs::create_dir_all(ctx.home_dir()?, Path::new(roots::BUILD_STAGING_DIRECTORY))?;
    let parent =
        gripsack_fs::open_dir_nofollow(ctx.home_dir()?, Path::new(roots::BUILD_STAGING_DIRECTORY))?;
    parent.create_dir(id)?;
    gripsack_fs::fsync_pinned_dir(&parent, Path::new("."))?;
    Ok(ctx.home.join(roots::BUILD_STAGING_DIRECTORY).join(id))
}
fn remove_staging(ctx: &Ctx, staging: &Path) -> Result<(), ExecError> {
    let parent =
        gripsack_fs::open_dir_nofollow(ctx.home_dir()?, Path::new(roots::BUILD_STAGING_DIRECTORY))?;
    parent.remove_dir_all(
        staging
            .file_name()
            .ok_or_else(|| failure("missing task staging basename"))?,
    )?;
    gripsack_fs::fsync_pinned_dir(&parent, Path::new("."))?;
    Ok(())
}
fn prepare(
    graph: &TaskGraph<'_>,
    ctx: &Ctx,
    options: &BuildOptions<'_>,
    realization: &realize::Realization<'_>,
    staging: &Path,
    facts: &gripsack_ir::HostFacts,
) -> Result<Vec<PreparedNode>, ExecError> {
    let host = admit::NativeContext::new(facts, &ctx.home, options.deadline)?;
    let checkout = live_checkout(ctx)?;
    let directory = gripsack_fs::open(staging)?;
    let mut prepared = Vec::with_capacity(graph.nodes.len());
    for (index, task) in graph.nodes.iter().enumerate() {
        let plan = match task.environment.as_deref() {
            Some(name) => {
                let WorkspaceOutput::Environment(environment) = graph.outputs[name] else {
                    unreachable!("admitted environment")
                };
                admit::EnvironmentPlan::admit(environment, &graph.outputs, realization, &host)?
            }
            None => admit::EnvironmentPlan::empty(),
        };
        directory.create_dir(index.to_string())?;
        let base = plan
            .materialize(&staging.join(index.to_string()), None)
            .map_err(operational)?;
        let prepare = |value, subject| -> Result<command::PreparedCommand, ExecError> {
            let mut command = command::prepare(
                value,
                subject,
                &plan,
                realization,
                &host,
                &checkout,
                options,
                &base,
            )?;
            command.overlay = plan.project_command_environment(&base, command.overlay)?;
            Ok(command)
        };
        let command_count = task
            .steps
            .iter()
            .filter(|step| matches!(step, WorkspaceStep::Command(_)))
            .count();
        let mut commands = Vec::with_capacity(command_count);
        for step in &task.steps {
            match step {
                WorkspaceStep::Command(command) => commands.push(prepare(command, None)?),
                WorkspaceStep::Action(
                    gripsack_ir::workspace_model::WorkspaceAction::EnsureArtifact { output, span },
                ) => {
                    if !realization.packages.contains_key(output.as_str()) {
                        return Err(gate(
                            span,
                            format!(
                                "EnsureArtifact {output:?} absent from common realized closure"
                            ),
                        ));
                    }
                }
            }
        }
        let mut checks = Vec::with_capacity(task.checks.len());
        for name in &task.checks {
            let WorkspaceOutput::Check(check) = graph.outputs[name.as_str()] else {
                unreachable!("admitted check")
            };
            checks.push(prepare(&check.run, Some(check.subject.as_str()))?);
        }
        prepared.push(PreparedNode { commands, checks });
    }
    Ok(prepared)
}

#[allow(clippy::too_many_arguments)]
fn execute(
    graph: &TaskGraph<'_>,
    nodes: Vec<PreparedNode>,
    ctx: &Ctx,
    options: &BuildOptions<'_>,
    policy: TaskRunPolicy,
    leases: &ProcessLeases,
    report: &mut TaskReport,
    logs: &mut Vec<u8>,
    launched: &mut bool,
) -> Result<(), ExecError> {
    let mut states = BTreeMap::new();
    let mut stop = false;
    for (task, node) in graph.nodes.iter().zip(nodes) {
        let ready = !stop
            && task.deps.iter().all(|name| {
                states
                    .get(name.as_str())
                    .copied()
                    .is_some_and(gripsack_policy::task::prerequisite_succeeded)
            });
        let index = report.nodes.len();
        report.nodes.push(TaskNodeReport {
            task: task.name.clone(),
            status: if ready {
                TaskStatus::Indeterminate
            } else {
                TaskStatus::Blocked
            },
            declared_steps: task.steps.len(),
            declared_checks: task.checks.len(),
            completed_steps: 0,
            completed_checks: 0,
            processes: Vec::new(),
        });
        report.persist(&ctx.home)?;
        if ready {
            let mut status = TaskStatus::Succeeded;
            // The admitted declaration supplies sequencing; retain only dense
            // command values, not a large tagged allocation for every ensure.
            let mut commands = node.commands.into_iter();
            let steps = task.steps.iter().map(move |step| {
                let command = match step {
                    WorkspaceStep::Command(_) => {
                        Some(commands.next().expect("declared command was prepared"))
                    }
                    WorkspaceStep::Action(_) => None,
                };
                (ActionPhase::Step, command)
            });
            for (phase, command) in steps.chain(
                node.checks
                    .into_iter()
                    .map(|command| (ActionPhase::Postcondition, Some(command))),
            ) {
                if Instant::now() >= options.deadline {
                    status = TaskStatus::Failed;
                    report.error =
                        Some("task invocation deadline exhausted before next action".into());
                    break;
                }
                if let Some(command) = command {
                    let invocation = Invocation::admit(
                        options.environment,
                        ProcessRole::Task,
                        &command.program,
                        &command.cwd,
                        consumer_limits(options.deadline),
                    )?
                    .with_overlay(command.overlay)
                    .retain_leases(clone_leases(leases)?)?;
                    let argv: Vec<_> = command.argv.iter().map(OsString::as_os_str).collect();
                    *launched = true;
                    let result = invocation.run_task(
                        &argv,
                        matches!(policy.stdin, TaskStdin::Inherit),
                        |bytes| {
                            let available = task_report::MAX_LOG_BYTES.saturating_sub(logs.len());
                            logs.extend_from_slice(&bytes[..bytes.len().min(available)]);
                            report.logs_truncated |= bytes.len() > available;
                            if matches!(policy.output, TaskOutputMode::Stream) {
                                let _ = std::io::stdout().write_all(bytes);
                            }
                            gripsack_process::Control::Continue
                        },
                    )?;
                    let available = task_report::MAX_LOG_BYTES.saturating_sub(logs.len());
                    logs.extend_from_slice(&result.stderr[..result.stderr.len().min(available)]);
                    report.logs_truncated |=
                        result.stderr_truncated || result.stderr.len() > available;
                    if matches!(policy.output, TaskOutputMode::Stream) {
                        let _ = std::io::stderr().write_all(&result.stderr);
                    }
                    status = TaskStatus::from_receipt(&result.receipt);
                    report.nodes[index].processes.push(result.receipt);
                    task_report::persist_log(&ctx.home, report, logs)?;
                    report.persist(&ctx.home)?;
                    if status != TaskStatus::Succeeded {
                        break;
                    }
                }
                match phase {
                    ActionPhase::Postcondition => report.nodes[index].completed_checks += 1,
                    ActionPhase::Step => report.nodes[index].completed_steps += 1,
                }
            }
            report.nodes[index].status = status;
            stop = matches!(status, TaskStatus::Indeterminate | TaskStatus::Cancelled);
        }
        states.insert(task.name.as_str(), report.nodes[index].status.state());
        report.persist(&ctx.home)?;
    }
    report.status = if report
        .nodes
        .iter()
        .any(|node| node.status == TaskStatus::Indeterminate)
    {
        TaskStatus::Indeterminate
    } else if report
        .nodes
        .iter()
        .any(|node| node.status == TaskStatus::Cancelled)
    {
        TaskStatus::Cancelled
    } else {
        report
            .nodes
            .last()
            .map_or(TaskStatus::Blocked, |node| node.status)
    };
    Ok(())
}
