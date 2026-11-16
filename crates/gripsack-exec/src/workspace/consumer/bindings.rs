//! Exact argument, working-directory and executable bindings shared by tasks,
//! checks and retained native launchers. Admission remains the single native path.
use super::*;
use crate::workspace::paths;
/// Resolve one argument value to exact bytes. Empty and spaced literals are
/// preserved verbatim; artifact/package/input bindings become absolute
/// store paths inside the realized closure.
pub(in crate::workspace) fn task_argument(
    argument: &WorkspaceArg,
    subject: Option<&str>,
    realization: &Realization,
    host: &admit::NativeContext<'_>,
    span: &Span,
) -> Result<OsString, ExecError> {
    Ok(match argument {
        WorkspaceArg::Literal { value } => OsString::from(value),
        WorkspaceArg::Artifact { output, selector } => path_argument(
            paths::resolve_artifact(output, selector, realization, span)?,
            span,
        )?,
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
        WorkspaceArg::Source { selector } => {
            let subject = subject.ok_or_else(|| gate(span, "task command has no check subject"))?;
            path_argument(
                paths::resolve_artifact(subject, selector, realization, span)?,
                span,
            )?
        }
        WorkspaceArg::Output { .. } => {
            return Err(gate(
                span,
                "production source/output bindings are unavailable to tasks",
            ));
        }
    })
}

pub(super) fn admit_package_command(
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
        && gripsack_ir::workspace_model::identity::ExecutableDigest::parse(claim)
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

pub(super) fn bind_admitted_program(
    command: &admit::AdmittedCommand,
    options: &crate::workspace::BuildOptions<'_>,
) -> Result<SelectedProgram, ExecError> {
    let selected = SelectedProgram::select(
        options.environment,
        &command.path,
        Some(gripsack_host::process::Sha256Digest::from_bytes(
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
    if let Some(loader) = &command.gnu_loader {
        let libraries = std::env::join_paths(&command.library_dirs).map_err(operational)?;
        selected
            .with_gnu_loader(std::sync::Arc::clone(loader), &libraries)
            .map_err(operational)
    } else {
        Ok(selected)
    }
}

fn path_argument(path: PathBuf, span: &Span) -> Result<OsString, ExecError> {
    if path.to_str().is_none() {
        return Err(gate(span, "path is not UTF-8"));
    }
    Ok(path.into_os_string())
}

pub(super) fn task_cwd(
    cwd: Option<&WorkspacePath>,
    subject: Option<&str>,
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
            paths::resolve_artifact(output, selector, realization, span)?
        }
        Some(WorkspacePath::Source { selector }) => {
            let subject = subject.ok_or_else(|| gate(span, "task command has no check subject"))?;
            paths::resolve_artifact(subject, selector, realization, span)?
        }
        Some(WorkspacePath::Output { .. }) => {
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
