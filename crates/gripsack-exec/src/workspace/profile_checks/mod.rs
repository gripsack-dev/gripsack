//! File validation uses the native command admission and existing activation
//! ledger. Only captured pre-flip checks run here; deployed pre-flip checks run
//! inside the caller's compensated transaction, and lifecycle checks are intents.
mod bindings;
mod execution;
mod subject;
#[cfg(test)]
mod tests;
use super::{consumer, content::PreparedFile, hooks, realize::Realization};
use crate::ExecError;
use gripsack_ir::{
    Span,
    workspace_model::{
        FileCheckStage, FileCheckSubject, ProfileOutput, WorkspaceFileCheck, WorkspaceOutput,
    },
};
use gripsack_process::{OperatorEnvironment, Sha256Digest};
use gripsack_store::ActivationTrigger;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};
pub(in crate::workspace) use subject::FileCheckInputs;

pub(super) struct Protection<'a> {
    pub session: &'a crate::LifecycleSession,
    pub closure: &'a super::roots::RetentionSet,
}

#[derive(Default)]
pub(super) struct PreparedChecks {
    pub hooks: Vec<hooks::PreparedHook>,
    pub pre_flip: Vec<PreparedCheck>,
}

pub(super) struct PreparedCheck {
    file: String,
    digest: Sha256Digest,
    span: Span,
}

impl PreparedCheck {
    /// Must be called after all destination writes, before the satisfied/flip
    /// decision, with errors routed through the existing journal compensation.
    pub(super) fn run(
        &self,
        home: &Path,
        profile: &Path,
        environment: &OperatorEnvironment,
        deadline: Instant,
        protection: &Protection<'_>,
    ) -> Result<(), ExecError> {
        let context = hooks::snapshot::read(home, &profile.join(&self.file), self.digest)?;
        execution::run(
            &context,
            profile,
            home,
            environment,
            deadline,
            &self.span,
            protection,
        )
    }
}

pub(super) fn admit(profile: &ProfileOutput) -> Result<(), ExecError> {
    for file in &profile.files {
        for check in &file.checks {
            if check.subject == FileCheckSubject::Source && file.source.is_none() {
                return Err(super::file_failure(
                    &check.span,
                    "a source file check requires a captured origin; literal-only content has no source",
                ));
            }
            // Epic A §3.3: new rendered-content validation must finish before
            // candidate deployment. Accepting a late stage would change that contract.
            if check.subject == FileCheckSubject::Rendered && check.stage != FileCheckStage::PreFlip
            {
                return Err(super::file_failure(
                    &check.span,
                    "rendered candidate checks require pre_flip; use deployed for post_link/post_activate validation",
                ));
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    checks: &[WorkspaceFileCheck],
    files: &[PreparedFile],
    outputs: &BTreeMap<&str, &WorkspaceOutput>,
    realization: &Realization,
    home: &Path,
    stage: &Path,
    host: &consumer::admit::NativeContext<'_>,
    deadline: Instant,
    resolve_destination: &impl Fn(&str) -> std::io::Result<PathBuf>,
    logical_source: &impl Fn(&str) -> String,
    prepared: &mut PreparedChecks,
    protection: &Protection<'_>,
) -> Result<(), ExecError> {
    if checks.is_empty() {
        return Ok(());
    }
    let environment = OperatorEnvironment::capture()?;
    let options = super::BuildOptions {
        environment: &environment,
        bridge: None,
        worker: Default::default(),
        deadline,
    };
    let retained_span = |span: &Span| Span {
        file: logical_source(&span.file),
        line: span.line,
        col: span.col,
    };
    for file in files {
        for check in checks {
            let Some(WorkspaceOutput::Check(declaration)) =
                outputs.get(check.check.as_str()).copied()
            else {
                return Err(super::file_failure(
                    &check.span,
                    format!("missing file check {:?}", check.check),
                ));
            };
            let captured = match check.subject {
                FileCheckSubject::Source => Path::new(&file.entry.from).with_file_name("source"),
                FileCheckSubject::Rendered | FileCheckSubject::Deployed => {
                    PathBuf::from(&file.entry.from)
                }
            };
            let path = stage.join(&captured);
            let inputs = FileCheckInputs {
                check: check.check.clone(),
                subject: check.subject,
                stage: check.stage,
                // Source coordinates participate in the snapshot identity, but
                // an evaluator's temporary capture location is not provenance.
                span: retained_span(&check.span),
                file_span: retained_span(file.entry.span.as_ref().unwrap_or(&check.span)),
                command_span: retained_span(declaration.run.span()),
                captured,
                captured_digest: gripsack_store::canonical_file_hash(&path)
                    .map_err(|error| super::file_failure(&check.span, error))?,
                destination: resolve_destination(&file.entry.to)?,
            };
            let bound = bindings::bind(&declaration.run, &path)?;
            let context = hooks::snapshot::capture_file(
                &declaration.run,
                &bound,
                inputs,
                realization,
                home,
                host,
                &options,
            )?;
            let bytes = context.encode()?;
            let digest = Sha256Digest::of(&bytes);
            // Content address, not a random staging name or invocation timestamp:
            // unchanged successful apply remains content-equal.
            let name = format!("file-check-{digest}.json");
            std::fs::write(stage.join(&name), bytes)?;
            match check.stage {
                FileCheckStage::PreFlip if check.subject != FileCheckSubject::Deployed => {
                    execution::run(
                        &context,
                        stage,
                        home,
                        &environment,
                        deadline,
                        &check.span,
                        protection,
                    )?;
                }
                FileCheckStage::PreFlip => prepared.pre_flip.push(PreparedCheck {
                    file: name,
                    digest,
                    span: check.span.clone(),
                }),
                FileCheckStage::PostLink => prepared.hooks.push(hooks::PreparedHook::new(
                    name,
                    digest,
                    ActivationTrigger::PostLink,
                )),
                FileCheckStage::PostActivate => prepared.hooks.push(hooks::PreparedHook::new(
                    name,
                    digest,
                    ActivationTrigger::PostActivate,
                )),
            }
        }
    }
    Ok(())
}
