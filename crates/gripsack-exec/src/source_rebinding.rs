//! Convert only declared source locations and typed native path fields back to
//! logical roots. Executable text, literal argv/env, templates and plugin data
//! are opaque bytes and must never undergo diagnostic string substitution.
use crate::ExecError;
use gripsack_ir::{
    FetchSpec, Ir, Span,
    workspace::WorkspaceDestination,
    workspace_model::{
        AcquisitionSource, InputOrigin, TaskContext, WorkspaceCommand, WorkspaceOutput,
        WorkspacePath, WorkspaceProducer, WorkspaceSource, WorkspaceStep,
    },
};
use gripsack_store::source_bundle::{SourceBundle, SourceRootKind};
use std::path::Path;

pub fn rebind_captured_ir(ir: &mut Ir, sources: &SourceBundle) -> Result<(), ExecError> {
    span(&mut ir.workspace.span, sources)?;
    for input in &mut ir.workspace.inputs {
        span(&mut input.span, sources)?;
        match &mut input.origin {
            InputOrigin::RepoFile { path: value }
            | InputOrigin::RepoDirectory { path: value, .. } => path(value, sources)?,
        }
    }
    for lock in &mut ir.workspace.mutation_locks {
        span(&mut lock.span, sources)?;
    }
    for output in &mut ir.workspace.outputs {
        match output {
            WorkspaceOutput::Profile(profile) => {
                span(&mut profile.span, sources)?;
                if let Some(launchers) = &mut profile.launchers {
                    path(&mut launchers.directory, sources)?;
                }
                for file in &mut profile.files {
                    span(&mut file.span, sources)?;
                    if let Some(
                        WorkspaceSource::RepoFile { path: value }
                        | WorkspaceSource::RepoTree { path: value, .. },
                    ) = &mut file.source
                    {
                        path(value, sources)?
                    }
                    match &mut file.destination {
                        WorkspaceDestination::Symlink { path: value }
                        | WorkspaceDestination::TrackedCopy { path: value }
                        | WorkspaceDestination::ManagedBlock { path: value, .. } => {
                            path(value, sources)?
                        }
                    }
                    for check in &mut file.checks {
                        span(&mut check.span, sources)?;
                    }
                }
            }
            WorkspaceOutput::Package(package) => {
                span(&mut package.span, sources)?;
                if let WorkspaceProducer::Provider { provider } = &mut package.producer {
                    source(provider, sources)?;
                }
            }
            WorkspaceOutput::Environment(environment) => span(&mut environment.span, sources)?,
            WorkspaceOutput::Task(task) => {
                span(&mut task.span, sources)?;
                for step in &mut task.steps {
                    step_locations(step, sources)?;
                }
                let TaskContext::Host { mutable_paths } = &mut task.context;
                for value in mutable_paths {
                    path(value, sources)?;
                }
                for lock in &mut task.mutation_locks {
                    span(&mut lock.span, sources)?;
                }
            }
            WorkspaceOutput::Check(check) => {
                span(&mut check.span, sources)?;
                command(&mut check.run, sources)?;
            }
            WorkspaceOutput::Hook(hook) => {
                span(&mut hook.span, sources)?;
                command(&mut hook.run, sources)?;
            }
            WorkspaceOutput::Schedule(schedule) => span(&mut schedule.span, sources)?,
            WorkspaceOutput::Service(service) => span(&mut service.span, sources)?,
            // Current authoring admission rejects historical producer variants.
            WorkspaceOutput::Recipe(recipe) => {
                span(&mut recipe.span, sources)?;
                source(&mut recipe.source, sources)?;
                for step in &mut recipe.steps {
                    step_locations(step, sources)?;
                }
            }
            WorkspaceOutput::Image(image) => span(&mut image.span, sources)?,
        }
    }
    Ok(())
}

fn source(source: &mut AcquisitionSource, sources: &SourceBundle) -> Result<(), ExecError> {
    match source {
        AcquisitionSource::Fetch(fetch) => {
            span(&mut fetch.span, sources)?;
            match &mut fetch.fetch {
                FetchSpec::File { path: value } => path(value, sources)?,
                FetchSpec::Tarball { url, .. } | FetchSpec::Git { url, .. }
                    if url.starts_with("file://") =>
                {
                    *url = sources.logical_text(url).into_owned();
                }
                _ => {}
            }
        }
        AcquisitionSource::CondaEnvironment(source) => span(&mut source.span, sources)?,
        AcquisitionSource::PixiLock(source) => span(&mut source.span, sources)?,
        AcquisitionSource::Package { span: location, .. } => span(location, sources)?,
    }
    Ok(())
}

fn step_locations(step: &mut WorkspaceStep, sources: &SourceBundle) -> Result<(), ExecError> {
    match step {
        WorkspaceStep::Command(value) => command(value, sources),
        WorkspaceStep::Action(gripsack_ir::workspace_model::WorkspaceAction::EnsureArtifact {
            span: location,
            ..
        }) => span(location, sources),
    }
}

fn command(command: &mut WorkspaceCommand, sources: &SourceBundle) -> Result<(), ExecError> {
    let (location, cwd) = match command {
        WorkspaceCommand::Exec { span, cwd, .. } | WorkspaceCommand::RunBash { span, cwd, .. } => {
            (span, cwd)
        }
    };
    span(location, sources)?;
    if let Some(WorkspacePath::Literal { value } | WorkspacePath::Host { path: value }) = cwd {
        path(value, sources)?;
    }
    Ok(())
}

fn span(span: &mut Span, sources: &SourceBundle) -> Result<(), ExecError> {
    if span.file.starts_with("file://") {
        span.file = sources.logical_text(&span.file).into_owned();
        Ok(())
    } else {
        path(&mut span.file, sources)
    }
}

fn path(value: &mut String, sources: &SourceBundle) -> Result<(), ExecError> {
    for kind in [
        SourceRootKind::PinnedFrontend,
        SourceRootKind::Frontend,
        SourceRootKind::Repository,
    ] {
        let (Some(captured), Some(original)) =
            (sources.captured_root(kind), sources.original_root(kind))
        else {
            continue;
        };
        let Ok(relative) = Path::new(value).strip_prefix(captured) else {
            continue;
        };
        let logical = original.join(relative);
        *value = logical.into_os_string().into_string().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "native source path is not UTF-8",
            )
        })?;
        break;
    }
    Ok(())
}
