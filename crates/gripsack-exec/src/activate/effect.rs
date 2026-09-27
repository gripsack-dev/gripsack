//! Native adapters share executable/environment admission and bounded process
//! supervision. Diagnostics retain identities/status, never script/argv bytes.
use super::output::CapturedOutput;
use crate::report::{ReportKind, StepReport};
use gripsack_ir::Action;
use gripsack_process::{
    ActivationEnvironment, Invocation, Limits, NativeInput, OperatorEnvironment, ProcessReceipt,
    ProcessRole,
};
use gripsack_store::activation::{AdmissionStage, EffectiveIntent, IntentFailure, LaunchPermit};
use std::{
    ffi::OsStr,
    io,
    num::NonZeroU64,
    path::{Path, PathBuf},
};

pub(super) struct EffectOutcome {
    pub processes: Vec<ProcessReceipt>,
    pub failure: Option<IntentFailure>,
    pub report: StepReport,
    pub output: CapturedOutput,
}

pub(super) fn run(
    intent: &EffectiveIntent,
    permit: &LaunchPermit,
    environment: &io::Result<OperatorEnvironment>,
    directory: &Path,
) -> EffectOutcome {
    let mut processes = Vec::new();
    let mut output = CapturedOutput::default();
    let result = (|| {
        let environment = environment
            .as_ref()
            .map_err(|error| admission(AdmissionStage::Environment, error))?;
        let activation = ActivationEnvironment {
            intent: permit.intent_id().digest(),
            attempt: NonZeroU64::new(permit.attempt().value()).ok_or_else(|| {
                admission(
                    AdmissionStage::Invocation,
                    &io::Error::from(io::ErrorKind::InvalidData),
                )
            })?,
        };
        let select = |program: &Path| {
            Invocation::admit(
                environment,
                ProcessRole::Hook,
                program,
                None,
                directory,
                Limits::default(),
            )
            .map_err(|error| admission(AdmissionStage::Invocation, &error))
        };
        match intent.action() {
            Action::CustomShell { script } => {
                let invocation = select(Path::new("/bin/sh"))?;
                let result = invocation
                    .run_shell_body(script, &activation, |bytes| output.stdout(bytes))
                    .map_err(|error| admission(AdmissionStage::Execution, &error))?;
                let succeeded = output.result(result, &mut processes);
                if !succeeded {
                    return Err(IntentFailure::Process);
                }
            }
            Action::Fonts | Action::DesktopEntry => {
                let fonts = matches!(intent.action(), Action::Fonts);
                let invocation = select(Path::new(if fonts {
                    "fc-cache"
                } else {
                    "update-desktop-database"
                }))?;
                execute(
                    &invocation,
                    &[OsStr::new("--version")],
                    &activation,
                    &mut processes,
                    &mut output,
                )?;
                if fonts {
                    execute(
                        &invocation,
                        &[OsStr::new("-f")],
                        &activation,
                        &mut processes,
                        &mut output,
                    )?;
                } else {
                    let applications = desktop_applications_dir();
                    execute(
                        &invocation,
                        &[applications.as_os_str()],
                        &activation,
                        &mut processes,
                        &mut output,
                    )?;
                }
            }
            Action::Service { name, user } => {
                let invocation = select(Path::new("systemctl"))?;
                let version = [OsStr::new("--user"), OsStr::new("--version")];
                let reload = [OsStr::new("--user"), OsStr::new("daemon-reload")];
                let enable = [
                    OsStr::new("--user"),
                    OsStr::new("enable"),
                    OsStr::new("--now"),
                    OsStr::new("--"),
                    OsStr::new(name),
                ];
                let start = usize::from(!user);
                execute(
                    &invocation,
                    &version[start..],
                    &activation,
                    &mut processes,
                    &mut output,
                )?;
                execute(
                    &invocation,
                    &reload[start..],
                    &activation,
                    &mut processes,
                    &mut output,
                )?;
                execute(
                    &invocation,
                    &enable[start..],
                    &activation,
                    &mut processes,
                    &mut output,
                )?;
            }
        }
        Ok(())
    })();
    let failure = result.err();
    let module = match intent.action() {
        Action::Fonts => "fonts".to_owned(),
        Action::DesktopEntry => "desktop-entry".to_owned(),
        _ => intent.contributors()[0].module.clone(),
    };
    let action = match intent.action() {
        Action::Fonts => "fontconfig cache refresh".to_owned(),
        Action::DesktopEntry => "desktop database refresh".to_owned(),
        Action::Service { name, .. } => format!("service {name}"),
        Action::CustomShell { .. } => "custom hook".to_owned(),
    };
    let summary = if failure.is_some() {
        format!(
            "{action} failed; intent {} attempt {} is recorded, no automatic retry",
            permit.intent_id(),
            permit.attempt().value()
        )
    } else {
        format!(
            "{action} completed; intent {} attempt {}",
            permit.intent_id(),
            permit.attempt().value()
        )
    };
    EffectOutcome {
        processes,
        failure,
        output,
        report: StepReport {
            module,
            summary,
            kind: if failure.is_some() {
                ReportKind::Warned
            } else {
                ReportKind::Configured
            },
        },
    }
}

fn execute(
    invocation: &Invocation<'_>,
    arguments: &[&OsStr],
    identity: &ActivationEnvironment,
    processes: &mut Vec<ProcessReceipt>,
    output: &mut CapturedOutput,
) -> Result<(), IntentFailure> {
    let result = invocation
        .run(
            arguments,
            NativeInput::Bytes(b""),
            Some(identity),
            |bytes| output.stdout(bytes),
        )
        .map_err(|error| admission(AdmissionStage::Execution, &error))?;
    let succeeded = output.result(result, processes);
    if succeeded {
        Ok(())
    } else {
        Err(IntentFailure::Process)
    }
}

fn desktop_applications_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("applications");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/share/applications");
    }
    PathBuf::from("~/.local/share/applications")
}

fn admission(stage: AdmissionStage, error: &io::Error) -> IntentFailure {
    IntentFailure::Admission {
        stage,
        error: gripsack_process::NativeIoError::capture(error),
    }
}
