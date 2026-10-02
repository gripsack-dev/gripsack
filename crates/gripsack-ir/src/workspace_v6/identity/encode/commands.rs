use super::{CommandPins, Encoder, PinGap};
use crate::workspace_v6::{
    WorkspaceAction, WorkspaceArg, WorkspaceCommand, WorkspacePath, WorkspaceStep,
};

pub(super) fn step(
    writer: &mut Encoder,
    step: &WorkspaceStep,
    pins: &CommandPins,
) -> Result<(), PinGap> {
    match step {
        WorkspaceStep::Command(value) => {
            writer.field(b"command");
            command(writer, value, pins)?;
        }
        WorkspaceStep::Action(WorkspaceAction::EnsureArtifact { output, .. }) => {
            writer.field(b"ensure-artifact");
            artifact(writer, output, pins)?;
        }
    }
    Ok(())
}
pub(super) fn command(
    writer: &mut Encoder,
    command: &WorkspaceCommand,
    pins: &CommandPins,
) -> Result<(), PinGap> {
    let (env, cwd) = match command {
        WorkspaceCommand::Exec { argv, env, cwd, .. } => {
            writer.field(b"exec");
            writer.number(argv.len() as u64);
            for value in argv {
                argument(writer, value, pins)?;
            }
            (env, cwd)
        }
        WorkspaceCommand::RunBash {
            interpreter,
            options,
            body,
            env,
            cwd,
            ..
        } => {
            writer.field(b"run-bash");
            argument(writer, interpreter, pins)?;
            writer.number(options.len() as u64);
            for option in options {
                writer.text(option);
            }
            writer.text(body);
            (env, cwd)
        }
    };
    writer.number(env.len() as u64);
    for (key, value) in env {
        writer.text(key);
        argument(writer, value, pins)?;
    }
    match cwd {
        None => writer.field(b"default-cwd"),
        Some(WorkspacePath::Literal { value }) => {
            writer.field(b"literal-cwd");
            writer.text(value);
        }
        Some(WorkspacePath::Host { path }) => {
            writer.field(b"host-cwd");
            writer.text(path);
        }
        Some(WorkspacePath::Source { selector }) => {
            writer.field(b"source-cwd");
            writer.text(selector);
        }
        Some(WorkspacePath::Output { selector }) => {
            writer.field(b"output-cwd");
            writer.text(selector);
        }
        Some(WorkspacePath::Artifact { output, selector }) => {
            writer.field(b"artifact-cwd");
            artifact(writer, output, pins)?;
            writer.text(selector);
        }
    }
    Ok(())
}
fn argument(writer: &mut Encoder, value: &WorkspaceArg, pins: &CommandPins) -> Result<(), PinGap> {
    match value {
        WorkspaceArg::Literal { value } => {
            writer.field(b"literal");
            writer.text(value);
        }
        WorkspaceArg::Source { selector } => {
            writer.field(b"source");
            writer.text(selector);
        }
        WorkspaceArg::Output { selector } => {
            writer.field(b"output");
            writer.text(selector);
        }
        WorkspaceArg::Artifact { output, selector } => {
            writer.field(b"artifact");
            artifact(writer, output, pins)?;
            writer.text(selector);
        }
        WorkspaceArg::Input { input } => {
            writer.field(b"captured-input");
            writer.field(
                pins.inputs
                    .get(input)
                    .ok_or_else(|| PinGap::Input(input.clone()))?
                    .bytes(),
            );
        }
        WorkspaceArg::PackageCommand {
            package,
            command,
            sha256,
        } => {
            let pin = pins
                .tools
                .get(package)
                .and_then(|commands| commands.get(command))
                .ok_or_else(|| PinGap::Tool {
                    package: package.clone(),
                    command: command.clone(),
                })?;
            writer.field(b"package-command");
            writer.field(pin.package.bytes());
            writer.text(&pin.selector);
            match pin.executable {
                crate::workspace_v6::identity::ToolExecutable::Captured(executable) => {
                    if let Some(claim) = sha256 && crate::workspace_v6::identity::ExecutableDigest::parse(claim)?
                            != executable {
                        return Err(PinGap::ToolMismatch);
                    }
                    writer.field(b"captured-executable");
                    writer.field(executable.bytes());
                }
                crate::workspace_v6::identity::ToolExecutable::Produced(production) => {
                    writer.field(b"produced-executable");
                    writer.field(production.bytes());
                    writer.optional(sha256);
                }
            }
        }
    }
    Ok(())
}
fn artifact(writer: &mut Encoder, output: &str, pins: &CommandPins) -> Result<(), PinGap> {
    writer.field(b"resolved-output");
    writer.field(
        pins.artifacts
            .get(output)
            .ok_or_else(|| PinGap::Artifact(output.to_owned()))?
            .bytes(),
    );
    Ok(())
}

pub(super) fn declaration(writer: &mut Encoder, command: &WorkspaceCommand) {
    let (env, cwd) = match command {
        WorkspaceCommand::Exec { argv, env, cwd, .. } => {
            writer.field(b"exec");
            writer.number(argv.len() as u64);
            for value in argv {
                declared_argument(writer, value);
            }
            (env, cwd)
        }
        WorkspaceCommand::RunBash {
            interpreter,
            options,
            body,
            env,
            cwd,
            ..
        } => {
            writer.field(b"run-bash");
            declared_argument(writer, interpreter);
            writer.number(options.len() as u64);
            for option in options {
                writer.text(option);
            }
            writer.text(body);
            (env, cwd)
        }
    };
    writer.number(env.len() as u64);
    for (key, value) in env {
        writer.text(key);
        declared_argument(writer, value);
    }
    match cwd {
        None => writer.field(b"default-cwd"),
        Some(WorkspacePath::Literal { value }) => {
            writer.field(b"literal-cwd");
            writer.text(value);
        }
        Some(WorkspacePath::Host { path }) => {
            writer.field(b"host-cwd");
            writer.text(path);
        }
        Some(WorkspacePath::Source { selector }) => {
            writer.field(b"source-cwd");
            writer.text(selector);
        }
        Some(WorkspacePath::Output { selector }) => {
            writer.field(b"output-cwd");
            writer.text(selector);
        }
        Some(WorkspacePath::Artifact { output, selector }) => {
            writer.field(b"artifact-cwd");
            writer.text(output);
            writer.text(selector);
        }
    }
}
pub(super) fn declared_argument(writer: &mut Encoder, value: &WorkspaceArg) {
    match value {
        WorkspaceArg::Literal { value } => {
            writer.field(b"literal");
            writer.text(value);
        }
        WorkspaceArg::Source { selector } => {
            writer.field(b"source");
            writer.text(selector);
        }
        WorkspaceArg::Output { selector } => {
            writer.field(b"output");
            writer.text(selector);
        }
        WorkspaceArg::Artifact { output, selector } => {
            writer.field(b"artifact");
            writer.text(output);
            writer.text(selector);
        }
        WorkspaceArg::PackageCommand {
            package,
            command,
            sha256,
        } => {
            writer.field(b"package-command");
            writer.text(package);
            writer.text(command);
            writer.optional(sha256);
        }
        WorkspaceArg::Input { input } => {
            writer.field(b"input");
            writer.text(input);
        }
    }
}
