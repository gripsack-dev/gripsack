//! Native project consumers and bounded invocation-local task graphs.
//! Consumers share realization, selected-executable admission and retained
//! process roots, never personal generations or a per-vertex build scheduler.
//! Project run/shell sessions remain interactive; tasks use finite supervised
//! invocations including their prerequisites and postconditions. The checkout
//! stays live and writable: host task access is not a hermetic sandbox.
pub(super) mod admit;
pub(super) mod command;
mod launcher;
pub use launcher::run_package_command;
mod task_graph;
mod task_locks;
mod task_report;
mod tasks;
pub use task_graph::{TaskPlan, TaskRevision, plan_task};
pub use task_report::{
    AttemptId, InvocationId, TaskExecutionBinding, TaskNodeReport, TaskReport, TaskStatus,
    read_task_log, read_task_report,
};
pub use tasks::{BusyPolicy, TaskOutputMode, TaskRunPolicy, TaskStdin, run_task};
mod bindings;
pub(in crate::workspace) use bindings::task_argument;
use bindings::{admit_package_command, bind_admitted_program, task_cwd};

use super::{
    realize::{self, Realization},
    roots::{self, ProcessLease, RetentionSet, RootId},
};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_ir::{
    Ir, Span,
    workspace_model::{WorkspaceArg, WorkspaceCommand, WorkspaceOutput, WorkspacePath},
};
use gripsack_process::{
    EnvironmentOverlay, Invocation, Limits, ProcessLeases, ProcessReceipt, ProcessRole,
    SelectedProgram,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

pub enum ConsumerRequest<'a> {
    Run {
        environment: &'a str,
        argv: Vec<OsString>,
    },
    /// The shell program is operator-selected at the CLI (its SHELL, else
    /// `sh`); the repository never names the operator's shell.
    Shell {
        environment: &'a str,
        shell: OsString,
    },
}

pub struct ConsumerOutcome {
    /// The supervised run/shell/package-command process receipt.
    pub receipt: ProcessReceipt,
    /// True when cleanup could not be confirmed: the process root stays
    /// registered rather than lying about retention.
    pub root_retained: bool,
}

pub fn consume(
    ir: &Ir,
    ctx: &Ctx,
    options: &super::BuildOptions<'_>,
    request: &ConsumerRequest<'_>,
) -> Result<ConsumerOutcome, ExecError> {
    let workspace = ir
        .workspace_catalog
        .as_ref()
        .ok_or_else(|| failure("native consumption requires an executable workspace catalog"))?;
    let host = admit::NativeContext::new(&ir.host, &ctx.home, options.deadline)?;
    let outputs: BTreeMap<&str, &WorkspaceOutput> = workspace
        .outputs
        .iter()
        .map(|output| (output.name(), output))
        .collect();
    let root_name = match request {
        ConsumerRequest::Run { environment, .. } | ConsumerRequest::Shell { environment, .. } => {
            environment
        }
    };
    if ctx.only.len() != 1 || ctx.only[0] != *root_name {
        return Err(failure(
            "consumer selection must name exactly its root output",
        ));
    }
    let root = outputs.get(root_name).ok_or_else(|| {
        gate(
            &workspace.span,
            format!("no workspace output named {root_name:?}"),
        )
    })?;
    let environment_name = match (request, root) {
        (
            ConsumerRequest::Run { .. } | ConsumerRequest::Shell { .. },
            WorkspaceOutput::Environment(_),
        ) => Some(*root_name),
        (ConsumerRequest::Run { .. } | ConsumerRequest::Shell { .. }, other) => {
            return Err(gate(
                other.span(),
                format!(
                    "run/shell selects an environment; {root_name:?} is {}",
                    other.kind()
                ),
            ));
        }
    };
    // The common realization service: cached/provider closures need no
    // bridge or worker; a cold production needs the explicitly selected
    // bridge. The held session carries through project-root registration so
    // no GC window opens between publication and root acquisition.
    let mut held = realize::realize_held(ir, ctx, options, &ctx.only)?;
    let (plan, environment_span) = match environment_name {
        Some(name) => {
            let WorkspaceOutput::Environment(declaration) = outputs[name] else {
                unreachable!("environment kind checked");
            };
            (
                admit::EnvironmentPlan::admit(declaration, &outputs, &held.realization, &host)?,
                declaration.span.clone(),
            )
        }
        None => (admit::EnvironmentPlan::empty(), workspace.span.clone()),
    };
    // The registered project environment selection: keyed by (worktree,
    // selection name) so two worktrees hold distinct roots over shared
    // immutable objects. Re-running a selection replaces exactly its own
    // root, releasing the previous closure to GC; a live older process is
    // protected by its own process lease.
    let project = roots::project_id(ctx.repository.identity())?;
    let selection_id = RootId::from_identity(
        &serde_json::to_vec(&(project.as_str(), root_name)).map_err(operational)?,
    );
    let mut closure = BTreeSet::new();
    for (_, paths) in &held.closures {
        closure.extend(paths.iter().cloned());
    }
    let retention = RetentionSet::admit(&held.session, closure)?;
    roots::register_project_root(&held.session, &selection_id, &retention)?;
    held.finish_build()?;
    let realize::HeldRealization { session, .. } = held;
    match request {
        ConsumerRequest::Run { argv, .. } => run_launch(
            ctx,
            session,
            &plan,
            &environment_span,
            argv,
            &live_checkout(ctx)?,
            options,
        ),
        ConsumerRequest::Shell { shell, .. } => {
            shell_launch(ctx, session, &plan, shell, &live_checkout(ctx)?, options)
        }
    }
}

fn run_launch(
    ctx: &Ctx,
    session: LifecycleSession,
    plan: &admit::EnvironmentPlan,
    span: &Span,
    argv: &[OsString],
    checkout: &Path,
    options: &super::BuildOptions<'_>,
) -> Result<ConsumerOutcome, ExecError> {
    let Some((program, arguments)) = argv.split_first() else {
        return Err(failure("run requires a command after `--`"));
    };
    let (program, admitted) = resolve_program(plan, program, span, options)?;
    with_process_root(ctx, session, plan.closure().clone(), |bin, leases| {
        let overlay = plan.materialize(bin, admitted).map_err(operational)?;
        let invocation = Invocation::admit(
            options.environment,
            ProcessRole::Task,
            &program,
            checkout,
            consumer_limits(options.deadline),
        )?
        .with_overlay(overlay)
        .retain_leases(leases)?;
        let borrowed: Vec<&OsStr> = arguments.iter().map(OsString::as_os_str).collect();
        invocation
            .run_interactive(&borrowed)
            .map_err(operational)
            .map(single_outcome)
    })
}

fn shell_launch(
    ctx: &Ctx,
    session: LifecycleSession,
    plan: &admit::EnvironmentPlan,
    shell: &OsStr,
    checkout: &Path,
    options: &super::BuildOptions<'_>,
) -> Result<ConsumerOutcome, ExecError> {
    let program = SelectedProgram::select(
        options.environment,
        Path::new(shell),
        None,
        options.deadline,
    )
    .map_err(operational)?;
    with_process_root(ctx, session, plan.closure().clone(), |bin, leases| {
        let overlay = plan.materialize(bin, None).map_err(operational)?;
        let invocation = Invocation::admit(
            options.environment,
            ProcessRole::Task,
            &program,
            checkout,
            consumer_limits(options.deadline),
        )?
        .with_overlay(overlay)
        .retain_leases(leases)?;
        invocation
            .run_interactive(&[])
            .map_err(operational)
            .map(single_outcome)
    })
}

/// Resolve a program name: an environment command first (its exact admitted
/// bytes and library closure), then an absolute path or an operator-PATH
/// name — the same order the composed PATH gives the child. Host programs
/// run as-is; only package commands carry layout admission.
fn resolve_program<'p>(
    plan: &'p admit::EnvironmentPlan,
    program: &OsStr,
    span: &Span,
    options: &super::BuildOptions<'_>,
) -> Result<(SelectedProgram, Option<&'p admit::AdmittedCommand>), ExecError> {
    let text = program
        .to_str()
        .ok_or_else(|| gate(span, "program name is not UTF-8"))?;
    if let Some(command) = plan.commands.get(text) {
        return Ok((bind_admitted_program(command, options)?, Some(command)));
    }
    let selected = SelectedProgram::select(
        options.environment,
        Path::new(program),
        None,
        options.deadline,
    )
    .map_err(|error| {
        gate(
            span,
            format!("program {text:?} is not an environment command and does not resolve: {error}"),
        )
    })?;
    Ok((selected, None))
}

fn single_outcome(outcome: gripsack_process::NativeOutcome) -> ConsumerOutcome {
    ConsumerOutcome {
        receipt: outcome.receipt,
        root_retained: false,
    }
}

/// Register the supervised process root before launch and release it only on
/// a confirmed exit receipt. The root's flock handle rides into the child,
/// so a dead supervisor cannot be mistaken for a dead consumer: GC and the
/// native recovery lane see the live holder. Unknown cleanup retains the
/// root and the staging directory.
fn with_process_root(
    ctx: &Ctx,
    session: LifecycleSession,
    closure: BTreeSet<PathBuf>,
    launch: impl FnOnce(&Path, ProcessLeases) -> Result<ConsumerOutcome, ExecError>,
) -> Result<ConsumerOutcome, ExecError> {
    let retention = RetentionSet::admit(&session, closure)?;
    let lease = ProcessLease::register(&session, &retention, None)?;
    let parent_path = Path::new(roots::BUILD_STAGING_DIRECTORY);
    gripsack_fs::create_dir_all(ctx.home_dir()?, parent_path)?;
    let parent = gripsack_fs::open_dir_nofollow(ctx.home_dir()?, parent_path)?;
    parent.create_dir(lease.id().as_str())?;
    let staging = gripsack_fs::open_dir_nofollow(&parent, Path::new(lease.id().as_str()))?;
    staging.create_dir("bin")?;
    gripsack_fs::fsync_pinned_dir(&parent, parent_path)?;
    let bin = ctx
        .home
        .join(roots::BUILD_STAGING_DIRECTORY)
        .join(lease.id().as_str())
        .join("bin");
    let handle = lease.duplicate_handle().map_err(operational)?;
    drop(session);
    let result = launch(
        &bin,
        ProcessLeases {
            worker: None,
            retention: Some(handle),
        },
    );
    let session = LifecycleSession::acquire(&ctx.home)?;
    // An unconfirmed launch failure leaves the root for lease-aware recovery.
    let mut outcome = result?;
    let lease_id = lease.id().as_str().to_owned();
    match lease.release(&session, &outcome.receipt) {
        Ok(()) => {
            if let Ok(parent) = gripsack_fs::open_dir_nofollow(ctx.home_dir()?, parent_path) {
                let _ = parent.remove_dir_all(Path::new(&lease_id));
                let _ = gripsack_fs::fsync_pinned_dir(&parent, parent_path);
            }
        }
        Err(error) => {
            outcome.root_retained = true;
            tracing::warn!(%error, "process root retained after unconfirmed cleanup");
        }
    }
    Ok(outcome)
}

fn clone_leases(leases: &ProcessLeases) -> Result<ProcessLeases, ExecError> {
    let duplicate = |file: &Option<std::fs::File>| {
        file.as_ref()
            .map(|file| file.try_clone())
            .transpose()
            .map_err(operational)
    };
    Ok(ProcessLeases {
        worker: duplicate(&leases.worker)?,
        retention: duplicate(&leases.retention)?,
    })
}

fn consumer_limits(deadline: std::time::Instant) -> Limits {
    Limits {
        operation_deadline: Some(deadline),
        ..Limits::default()
    }
}

fn live_checkout(ctx: &Ctx) -> Result<PathBuf, ExecError> {
    let checkout = ctx.repository.identity();
    if !checkout.is_dir() {
        return Err(failure("the live repository checkout is not a directory"));
    }
    Ok(checkout.to_path_buf())
}

fn absolute_directories(value: &str, span: &Span, key: &str) -> Result<Vec<PathBuf>, ExecError> {
    let mut directories = Vec::new();
    for entry in value.split(':').filter(|entry| !entry.is_empty()) {
        let path = PathBuf::from(entry);
        if !path.is_absolute() {
            return Err(gate(
                span,
                format!("declared {key} entry {entry:?} is not absolute"),
            ));
        }
        directories.push(path);
    }
    Ok(directories)
}

fn gate(span: &Span, detail: impl Into<String>) -> ExecError {
    admit::gate(span, detail)
}

fn failure(detail: &str) -> ExecError {
    admit::failure(detail)
}

fn operational(error: impl std::fmt::Display) -> ExecError {
    admit::operational(error)
}
