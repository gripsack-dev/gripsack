//! Workspace declarations enter the existing generation/activation protocol.
//! The retained context is immutable input evidence, not launch authority:
//! replay verifies it and repeats native executable/layout admission.
mod command;
pub(super) mod snapshot;

use super::{consumer, realize::Realization};
use crate::ExecError;
use gripsack_ir::{
    Span, Trigger,
    workspace_model::{HookTrigger, WorkspaceOutput},
};
use gripsack_process::{
    ActivationEnvironment, Invocation, Limits, NativeInput, OperatorEnvironment, ProcessRole,
    Sha256Digest,
};
use gripsack_store::{self as store, activation::ActivationAction};
use std::{collections::BTreeMap, ffi::OsString, io, path::Path, time::Instant};

pub(super) struct PreparedHook {
    file: String,
    digest: Sha256Digest,
    trigger: Trigger,
}
impl PreparedHook {
    pub(super) fn new(file: String, digest: Sha256Digest, trigger: Trigger) -> Self {
        Self {
            file,
            digest,
            trigger,
        }
    }
    pub(super) fn record(self, profile: &Path) -> store::IntentRecord {
        store::IntentRecord {
            action: ActivationAction::WorkspaceHook {
                context: profile.join(self.file),
                sha256: self.digest,
            },
            trigger: self.trigger,
        }
    }
}

pub(super) fn prepare(
    names: &[String],
    outputs: &BTreeMap<&str, &WorkspaceOutput>,
    realization: &Realization,
    home: &Path,
    stage: &Path,
    host: &consumer::admit::NativeContext<'_>,
    deadline: Instant,
) -> Result<Vec<PreparedHook>, ExecError> {
    let environment = OperatorEnvironment::capture()?;
    let options = super::BuildOptions {
        environment: &environment,
        bridge: None,
        worker: Default::default(),
        deadline,
    };
    let mut prepared = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let Some(WorkspaceOutput::Hook(hook)) = outputs.get(name.as_str()).copied() else {
            return Err(super::file_failure(
                &Span {
                    file: "<workspace-hook>".into(),
                    line: 1,
                    col: None,
                },
                format!("missing hook {name:?}"),
            ));
        };
        let context = snapshot::capture(&hook.run, realization, home, host, &options)?;
        let bytes = context
            .encode()
            .map_err(|error| super::file_failure(&hook.span, error))?;
        let file = format!("hook-{index}.json");
        std::fs::write(stage.join(&file), &bytes)?;
        prepared.push(PreparedHook {
            file,
            digest: Sha256Digest::of(&bytes),
            trigger: match hook.trigger {
                HookTrigger::PostLink => Trigger::PostLink,
                HookTrigger::PostActivate => Trigger::PostActivate,
                HookTrigger::OnRemove => Trigger::OnRemove,
            },
        });
    }
    Ok(prepared)
}

pub(crate) fn run(
    path: &Path,
    digest: Sha256Digest,
    home: &Path,
    activation: &ActivationEnvironment,
    environment: &OperatorEnvironment,
    stdout: impl FnMut(&[u8]) -> gripsack_process::Control,
) -> io::Result<gripsack_process::NativeOutcome> {
    let context = snapshot::read(home, path, digest).map_err(io::Error::other)?;
    let root = path
        .parent()
        .ok_or_else(|| io::Error::other("hook context has no root"))?;
    run_context(
        &context,
        root,
        home,
        Some(activation),
        environment,
        Instant::now() + Limits::default().timeout,
        None,
        stdout,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_context(
    context: &snapshot::Context,
    root: &Path,
    home: &Path,
    activation: Option<&ActivationEnvironment>,
    environment: &OperatorEnvironment,
    deadline: Instant,
    leases: Option<gripsack_process::ProcessLeases>,
    stdout: impl FnMut(&[u8]) -> gripsack_process::Control,
) -> io::Result<gripsack_process::NativeOutcome> {
    let options = super::BuildOptions {
        environment,
        bridge: None,
        worker: Default::default(),
        deadline,
    };
    let observation = context
        .file_check
        .as_ref()
        .map(|inputs| inputs.observe(root))
        .transpose()
        .map_err(io::Error::other)?;
    let subject = observation
        .as_ref()
        .map(|value| {
            value
                .path()
                .to_str()
                .ok_or_else(|| io::Error::other("file-check subject is not UTF-8"))
        })
        .transpose()?;
    let facts = crate::facts::detect();
    let facts = gripsack_ir::HostFacts {
        os: facts.os.into(),
        arch: facts.arch.into(),
        libc: facts.libc.clone(),
        tags: Vec::new(),
    };
    let host = consumer::admit::NativeContext::new(&facts, home, options.deadline)
        .map_err(io::Error::other)?;
    let realization = context.realization(home).map_err(io::Error::other)?;
    let base = gripsack_process::EnvironmentOverlay::admit(Vec::new(), Vec::new(), Vec::new())?;
    let mut command = context.command.decode(subject).map_err(io::Error::other)?;
    if let Some(inputs) = &context.file_check {
        match &mut command {
            gripsack_ir::workspace_model::WorkspaceCommand::Exec { span, .. }
            | gripsack_ir::workspace_model::WorkspaceCommand::RunBash { span, .. } => {
                *span = inputs.command_span.clone();
            }
        }
    }
    let prepared = consumer::command::prepare(
        &command,
        None,
        &consumer::admit::EnvironmentPlan::empty(),
        &realization,
        &host,
        &std::env::temp_dir(),
        &options,
        &base,
    )
    .map_err(io::Error::other)?;
    if prepared.program.identity() != context.program {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "retained hook executable identity changed",
        ));
    }
    let invocation = Invocation::admit(
        environment,
        if context.file_check.is_some() && activation.is_none() {
            ProcessRole::Verify
        } else {
            ProcessRole::Hook
        },
        &prepared.program,
        &prepared.cwd,
        Limits {
            operation_deadline: Some(options.deadline),
            ..Limits::default()
        },
    )?
    .with_overlay(prepared.overlay);
    let invocation = match leases {
        Some(leases) => invocation.retain_leases(leases)?,
        None => invocation,
    };
    let argv: Vec<_> = prepared.argv.iter().map(OsString::as_os_str).collect();
    let outcome = if context.file_check.is_some() && activation.is_none() {
        invocation.run_verification(&argv, stdout)?
    } else {
        invocation.run(&argv, NativeInput::Bytes(b""), activation, stdout)?
    };
    if let Some(observation) = observation {
        observation.unchanged()?;
    }
    Ok(outcome)
}
