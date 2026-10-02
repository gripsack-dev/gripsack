//! Native project consumers: `run`, `shell` and basic single-command tasks.
//! One launch path serves all three: realize the selected closure through the
//! common service (no consumer-caused rebuild and never a personal
//! generation), admit host target + executable layout, register a supervised
//! process root that survives parent death and GC, then launch through the
//! shared selected-executable/exec-payload/interactive admission. The
//! checkout stays live and writable; host task access is not hermetic.
pub(super) mod admit;

use super::{
    realize::{self, Realization},
    roots::{self, ProcessLease, RetentionSet, RootId},
};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_ir::{
    Diagnostic, Ir, Span, codes,
    workspace_v6::{
        TaskContext, TaskOutput, WorkspaceArg, WorkspaceCommand, WorkspaceOutput, WorkspacePath,
        WorkspaceStep,
    },
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
    Task {
        task: &'a str,
    },
}

pub struct ConsumerOutcome {
    /// The last launched process receipt. Task steps run in declaration
    /// order and stop at the first failure, whose receipt this is.
    pub receipt: ProcessReceipt,
    /// Steps completed before this outcome (0 for run/shell).
    pub completed_steps: usize,
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
        .workspace_v6
        .as_ref()
        .ok_or_else(|| failure("native consumption requires a current v6 workspace"))?;
    let host = admit::NativeContext::new(&ir.host, &ctx.home)?;
    let outputs: BTreeMap<&str, &WorkspaceOutput> = workspace
        .outputs
        .iter()
        .map(|output| (output.name(), output))
        .collect();
    let root_name = match request {
        ConsumerRequest::Run { environment, .. } | ConsumerRequest::Shell { environment, .. } => {
            environment
        }
        ConsumerRequest::Task { task } => task,
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
        (ConsumerRequest::Task { .. }, WorkspaceOutput::Task(task)) => {
            admit_task_shape(task)?;
            task.environment.as_deref()
        }
        (ConsumerRequest::Task { .. }, other) => {
            return Err(gate(
                other.span(),
                format!(
                    "task selects a task output; {root_name:?} is {}",
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
    let realize::HeldRealization {
        realization,
        session,
        ..
    } = held;
    let checkout = live_checkout(ctx)?;
    match request {
        ConsumerRequest::Run { argv, .. } => run_launch(
            ctx,
            session,
            &plan,
            &environment_span,
            &host,
            &realization,
            argv,
            &checkout,
            options,
        ),
        ConsumerRequest::Shell { shell, .. } => {
            shell_launch(ctx, session, &plan, shell, &checkout, options)
        }
        ConsumerRequest::Task { task } => {
            let WorkspaceOutput::Task(declaration) = outputs[task] else {
                unreachable!("task kind checked");
            };
            task_launch(
                ctx,
                session,
                declaration,
                &plan,
                &realization,
                &host,
                &checkout,
                options,
            )
        }
    }
}

/// Explicit, named refusals for task shapes owned by later lanes. Nothing
/// here is silently skipped.
fn admit_task_shape(task: &TaskOutput) -> Result<(), ExecError> {
    if !task.deps.is_empty() {
        return Err(unavailable(
            &task.span,
            "task prerequisite execution belongs to E1; invoke prerequisites separately",
        ));
    }
    if !task.checks.is_empty() {
        return Err(unavailable(
            &task.span,
            "per-invocation task postconditions belong to E1; run the named checks as tasks",
        ));
    }
    if !task.mutation_locks.is_empty() {
        return Err(unavailable(
            &task.span,
            "mutation-lock enforcement belongs to its executor lane",
        ));
    }
    let TaskContext::Host { .. } = &task.context;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_launch(
    ctx: &Ctx,
    session: LifecycleSession,
    plan: &admit::EnvironmentPlan,
    span: &Span,
    host: &admit::NativeContext<'_>,
    realization: &Realization,
    argv: &[OsString],
    checkout: &Path,
    options: &super::BuildOptions<'_>,
) -> Result<ConsumerOutcome, ExecError> {
    let Some((program, arguments)) = argv.split_first() else {
        return Err(failure("run requires a command after `--`"));
    };
    let (program, admitted) = resolve_program(plan, realization, host, program, span, options)?;
    let arguments: Vec<OsString> = arguments.to_vec();
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

fn task_launch(
    ctx: &Ctx,
    session: LifecycleSession,
    task: &TaskOutput,
    plan: &admit::EnvironmentPlan,
    realization: &Realization,
    host: &admit::NativeContext<'_>,
    checkout: &Path,
    options: &super::BuildOptions<'_>,
) -> Result<ConsumerOutcome, ExecError> {
    let mut completed = 0usize;
    let outcome = with_process_root(ctx, session, plan.closure().clone(), |bin, leases| {
        let base_overlay = plan.materialize(bin, None).map_err(operational)?;
        let mut last: Option<ConsumerOutcome> = None;
        for step in &task.steps {
            match step {
                WorkspaceStep::Action(action) => {
                    let gripsack_ir::workspace_v6::WorkspaceAction::EnsureArtifact { output, span } =
                        action;
                    if !realization.recipes.contains_key(output.as_str())
                        && !realization.packages.contains_key(output.as_str())
                    {
                        return Err(gate(
                            span,
                            format!(
                                "ensure_artifact names {output:?}, which is not in this invocation's realized closure"
                            ),
                        ));
                    }
                }
                WorkspaceStep::Command(command) => {
                    let outcome = task_command(
                        command,
                        plan,
                        realization,
                        host,
                        checkout,
                        options,
                        &base_overlay,
                        &leases,
                    )?;
                    let success = outcome.success;
                    last = Some(ConsumerOutcome {
                        receipt: outcome.receipt,
                        completed_steps: completed,
                        root_retained: false,
                    });
                    if !success {
                        return Ok(last.expect("recorded"));
                    }
                }
            }
            completed += 1;
        }
        last.ok_or_else(|| failure("task produced no process outcome"))
    });
    outcome.map(|mut outcome| {
        outcome.completed_steps = completed;
        outcome
    })
}

#[allow(clippy::too_many_arguments)]
fn task_command(
    command: &WorkspaceCommand,
    plan: &admit::EnvironmentPlan,
    realization: &Realization,
    host: &admit::NativeContext<'_>,
    checkout: &Path,
    options: &super::BuildOptions<'_>,
    base_overlay: &EnvironmentOverlay,
    leases: &ProcessLeases,
) -> Result<gripsack_process::NativeOutcome, ExecError> {
    let span = command.span().clone();
    let (program_binding, argv, declared_env) = match command {
        WorkspaceCommand::Exec { argv, env, .. } => {
            let (program, arguments) = argv
                .split_first()
                .ok_or_else(|| gate(&span, "command has no program"))?;
            let mut resolved = Vec::with_capacity(arguments.len());
            for argument in arguments {
                resolved.push(task_argument(argument, realization, host, &span)?);
            }
            (program, resolved, env)
        }
        WorkspaceCommand::RunBash {
            interpreter,
            options: strict,
            body,
            env,
            ..
        } => {
            // Mirror the production lowering: interpreter, the fixed strict
            // option set, `-c`, body, then the stable $0 label.
            let mut resolved = Vec::with_capacity(strict.len() + 3);
            resolved.extend(strict.iter().map(OsString::from));
            resolved.push(OsString::from("-c"));
            resolved.push(OsString::from(body));
            resolved.push(OsString::from("gripsack-bash"));
            (interpreter, resolved, env)
        }
    };
    let mut entries: Vec<(OsString, OsString)> = base_overlay
        .entries()
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let mut library: Vec<PathBuf> = base_overlay.library_prefix().to_vec();
    // Command-declared PATH dirs precede the environment's declared segment
    // and the operator PATH, after the exported-commands bin directory.
    let mut search: Vec<PathBuf> = Vec::new();
    for (key, argument) in declared_env {
        let value = task_argument(argument, realization, host, &span)?;
        let text = value
            .to_str()
            .ok_or_else(|| gate(&span, "environment value is not UTF-8"))?
            .to_owned();
        match key.as_str() {
            "PATH" => search.extend(absolute_directories(&text, &span, "PATH")?),
            "LD_LIBRARY_PATH" => {
                library.extend(absolute_directories(&text, &span, "LD_LIBRARY_PATH")?)
            }
            _ => entries.push((OsString::from(key), value)),
        }
    }
    search.extend(base_overlay.search_prefix().iter().cloned());
    let explicit = match program_binding {
        WorkspaceArg::PackageCommand {
            package,
            command,
            sha256,
        } => Some(admit_package_command(
            package,
            command,
            sha256.as_deref(),
            realization,
            host,
            &span,
        )?),
        _ => None,
    };
    let (program, admitted) = if let Some(command) = explicit.as_ref() {
        (bind_admitted_program(command, options)?, Some(command))
    } else {
        let program_arg = task_argument(program_binding, realization, host, &span)?;
        resolve_program(plan, realization, host, &program_arg, &span, options)?
    };
    if let Some(admitted) = admitted {
        let mut dirs = admitted.library_dirs.clone();
        dirs.extend(library);
        library = dirs;
    }
    admit::bytecode::apply(
        admitted
            .and_then(|command| command.bytecode)
            .or(plan.bytecode),
        &mut entries,
        &span,
    )?;
    let overlay = EnvironmentOverlay::admit(entries, search, library).map_err(operational)?;
    let cwd = task_cwd(command.working_directory(), realization, checkout, &span)?;
    let invocation = Invocation::admit(
        options.environment,
        ProcessRole::Task,
        &program,
        &cwd,
        consumer_limits(options.deadline),
    )?
    .with_overlay(overlay)
    .retain_leases(clone_leases(leases)?)?;
    let borrowed: Vec<&OsStr> = argv.iter().map(OsString::as_os_str).collect();
    invocation.run_interactive(&borrowed).map_err(operational)
}

/// Resolve one argument value to exact bytes. Empty and spaced literals are
/// preserved verbatim; artifact/package/input bindings become absolute
/// store paths inside the realized closure.
fn task_argument(
    argument: &WorkspaceArg,
    realization: &Realization,
    host: &admit::NativeContext<'_>,
    span: &Span,
) -> Result<OsString, ExecError> {
    Ok(match argument {
        WorkspaceArg::Literal { value } => OsString::from(value),
        WorkspaceArg::Artifact { output, selector } => {
            let payload = admit::artifact_payload(output, realization)
                .ok_or_else(|| gate(span, format!("artifact {output:?} is not realized")))?;
            OsString::from(admit::utf8_path(
                &inside_payload(&payload, selector, span)?,
                span,
            )?)
        }
        WorkspaceArg::PackageCommand {
            package,
            command,
            sha256,
        } => {
            let admitted = admit_package_command(
                package,
                command,
                sha256.as_deref(),
                realization,
                host,
                span,
            )?;
            OsString::from(admit::utf8_path(&admitted.path, span)?)
        }
        WorkspaceArg::Input { input } => {
            let path = realization
                .inputs
                .get(input.as_str())
                .ok_or_else(|| gate(span, format!("input {input:?} is not captured")))?;
            OsString::from(admit::utf8_path(path, span)?)
        }
        WorkspaceArg::Source { .. } | WorkspaceArg::Output { .. } => {
            return Err(gate(
                span,
                "production source/output bindings are unavailable to tasks",
            ));
        }
    })
}

fn admit_package_command(
    package: &str,
    command: &str,
    claim: Option<&str>,
    realization: &Realization,
    host: &admit::NativeContext<'_>,
    span: &Span,
) -> Result<admit::AdmittedCommand, ExecError> {
    let realized = realization.packages.get(package).ok_or_else(|| {
        gate(
            span,
            format!("package {package:?} is not in the realized closure"),
        )
    })?;
    let provided = realized.commands.get(command).ok_or_else(|| {
        gate(
            span,
            format!("package {package:?} does not export command {command:?}"),
        )
    })?;
    if let Some(claim) = claim
        && gripsack_ir::workspace_v6::identity::ExecutableDigest::parse(claim)
            .map(|claim| claim != provided.executable)
            .unwrap_or(true)
    {
        return Err(gate(
            span,
            format!(
                "package command {command:?} digest claim differs from its publication receipt"
            ),
        ));
    }
    admit::admit_command(
        realized,
        &provided.selector,
        &provided.executable,
        host,
        span,
    )
}

fn bind_admitted_program(
    command: &admit::AdmittedCommand,
    options: &super::BuildOptions<'_>,
) -> Result<SelectedProgram, ExecError> {
    let selected = SelectedProgram::select(
        options.environment,
        &command.path,
        Some(gripsack_process::Sha256Digest::from_bytes(
            *command.executable.bytes(),
        )),
        options.deadline,
    )
    .map_err(operational)?;
    if let Some(interpreter) = &command.interpreter
        && (!selected.is_script()
            || selected.identity().executable_sha256 != interpreter.executable)
    {
        return Err(failure(
            "selected package interpreter differs from its admitted bytes",
        ));
    }
    if let Some(loader) = command.gnu_loader {
        let libraries = std::env::join_paths(&command.library_dirs).map_err(operational)?;
        selected
            .with_gnu_loader(
                options.environment,
                Path::new(loader),
                &libraries,
                options.deadline,
            )
            .map_err(operational)
    } else if let Some(directories) = &command.macho_library_dirs {
        selected
            .with_macho_libraries(directories)
            .map_err(operational)
    } else {
        Ok(selected)
    }
}

fn inside_payload(payload: &Path, selector: &str, span: &Span) -> Result<PathBuf, ExecError> {
    let canonical = payload.join(selector).canonicalize().map_err(|_| {
        gate(
            span,
            format!("artifact selector {selector:?} does not resolve"),
        )
    })?;
    if !canonical.starts_with(payload) {
        return Err(gate(span, "artifact selector escapes its payload"));
    }
    Ok(canonical)
}

fn task_cwd(
    cwd: Option<&WorkspacePath>,
    realization: &Realization,
    checkout: &Path,
    span: &Span,
) -> Result<PathBuf, ExecError> {
    let candidate = match cwd {
        // The default is the live, writable checkout — development tasks
        // never run against a frozen capture.
        None => checkout.to_path_buf(),
        Some(WorkspacePath::Literal { value }) => {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                checkout.join(path)
            }
        }
        Some(WorkspacePath::Host { path }) => {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(gate(span, "host working directory must be absolute"));
            }
            path
        }
        Some(WorkspacePath::Artifact { output, selector }) => {
            let payload = admit::artifact_payload(output, realization)
                .ok_or_else(|| gate(span, format!("artifact {output:?} is not realized")))?;
            payload.join(selector)
        }
        Some(WorkspacePath::Source { .. } | WorkspacePath::Output { .. }) => {
            return Err(gate(
                span,
                "production source/output directories are unavailable to tasks",
            ));
        }
    };
    if !candidate.is_dir() {
        return Err(gate(
            span,
            format!(
                "working directory {} is not a directory",
                candidate.display()
            ),
        ));
    }
    Ok(candidate)
}

/// Resolve a program name: an environment command first (its exact admitted
/// bytes and library closure), then an absolute path or an operator-PATH
/// name — the same order the composed PATH gives the child. Host programs
/// run as-is; only package commands carry layout admission.
fn resolve_program<'p>(
    plan: &'p admit::EnvironmentPlan,
    realization: &Realization,
    host: &admit::NativeContext<'_>,
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
    let _ = realization;
    let _ = host;
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
        completed_steps: 0,
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
    let mut outcome = match result {
        Ok(outcome) => outcome,
        // Admission or launch failure before a confirmed exit cannot release
        // the root honestly; the recovery lane retires it once no holder
        // remains. The original error surfaces unchanged.
        Err(error) => return Err(error),
    };
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

fn unavailable(span: &Span, detail: impl Into<String>) -> ExecError {
    ExecError::Gate(
        Diagnostic::error(codes::WORKSPACE_EXEC_UNAVAILABLE, detail.into())
            .with_label(Some(span.clone()), "unavailable task shape declared here"),
    )
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
