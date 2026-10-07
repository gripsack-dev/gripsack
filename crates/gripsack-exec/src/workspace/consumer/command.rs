//! One native command resolution path for task/check invocation and retained
//! hook replay. Admission remeasures native facts and seals executable bytes.
use super::*;

pub(in crate::workspace) struct PreparedCommand {
    pub program: SelectedProgram,
    pub argv: Vec<OsString>,
    pub overlay: EnvironmentOverlay,
    pub cwd: PathBuf,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::workspace) fn prepare(
    command: &WorkspaceCommand,
    subject: Option<&str>,
    plan: &admit::EnvironmentPlan,
    realization: &Realization,
    host: &admit::NativeContext<'_>,
    checkout: &Path,
    options: &crate::workspace::BuildOptions<'_>,
    base_overlay: &EnvironmentOverlay,
) -> Result<PreparedCommand, ExecError> {
    let span = command.span().clone();
    let (program_binding, argv, declared_env) = match command {
        WorkspaceCommand::Exec { argv, env, .. } => {
            let (program, arguments) = argv
                .split_first()
                .ok_or_else(|| gate(&span, "command has no program"))?;
            let mut resolved = Vec::with_capacity(arguments.len());
            for argument in arguments {
                resolved.push(task_argument(argument, subject, realization, host, &span)?);
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
    let mut search: Vec<PathBuf> = base_overlay
        .search_prefix()
        .iter()
        .take(1)
        .cloned()
        .collect();
    for (key, argument) in declared_env {
        let value = task_argument(argument, subject, realization, host, &span)?;
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
    search.extend(base_overlay.search_prefix().iter().skip(1).cloned());
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
        let program_arg = task_argument(program_binding, subject, realization, host, &span)?;
        resolve_program(plan, &program_arg, &span, options)?
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
    let cwd = task_cwd(command.working_directory(), subject, realization, checkout, &span)?;
    Ok(PreparedCommand { program, argv, overlay, cwd })
}
