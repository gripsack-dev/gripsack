//! Native preparation and identity calculation for one selected output closure.
//! Production identities and captured command pins precede final package
//! identities so native runtime dependencies retain complete authority.
use super::{
    acquire::{self, ResolutionMode},
    artifact::{self, Artifact},
    definitions::captured_definitions,
    inputs::{self, CapturedInput},
    pins::WorkspacePins,
    selection::Selection,
};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_ir::{
    Diagnostic, Ir, Span, codes,
    workspace_model::{
        identity::{
            self, ArtifactDigest, CommandPins, PackageDigest, PinGap, ToolExecutable, ToolPin,
        },
        lock::ResolvedPinFields,
        *,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    sync::Arc,
};

pub(super) struct PreparedSource {
    pub artifact: Arc<Artifact>,
    pub resolved: ResolvedPinFields,
    /// Conda runtime receipt for prefix-materialized provider packages.
    pub conda: Option<super::conda::CondaRuntimeReceipt>,
}
pub(super) struct Prepared<'a> {
    pub selection: Selection<'a>,
    pub sources: BTreeMap<&'a str, PreparedSource>,
    pub inputs: BTreeMap<&'a str, CapturedInput>,
    pub packages: BTreeMap<String, PackageDigest>,
}

impl<'a> Prepared<'a> {
    pub fn frozen(
        ir: &'a Ir,
        ctx: &Ctx,
        session: &LifecycleSession,
        requested: &[String],
    ) -> Result<Self, ExecError> {
        if let Some(diagnostic) = gripsack_ir::sema::run(ir)
            .into_iter()
            .find(|diagnostic| diagnostic.severity == gripsack_ir::Severity::Error)
        {
            return Err(ExecError::Gate(diagnostic));
        }
        let workspace = &ir.workspace;
        let selection = Selection::admit(workspace, requested)?;
        admit_capabilities(&selection)?;
        let definitions = captured_definitions(&ctx.repository)?;
        let pins = WorkspacePins::read(&ctx.repository)?;
        pins.admit_definitions(&definitions)?;
        let mut sources = BTreeMap::new();
        for name in &selection.required {
            let output = selection.outputs[name];
            let source = match output {
                WorkspaceOutput::Package(PackageOutput {
                    producer: WorkspaceProducer::Provider { provider },
                    target,
                    ..
                }) => Some((provider, target)),
                _ => None,
            };
            if let Some((source, platform)) = source {
                let pin = pins.lookup(platform, name, &source.locked())?;
                let prepared = match source {
                    AcquisitionSource::Fetch(fetch) => {
                        let acquired =
                            acquire::acquire(ctx, name, fetch, ResolutionMode::Frozen(pin))?
                                .publish(ctx, session, name)?;
                        let acquire::AcquiredSource {
                            path,
                            tree,
                            resolved,
                        } = acquired;
                        PreparedSource {
                            artifact: Arc::new(Artifact {
                                payload: path.clone(),
                                retention: BTreeSet::from([path.clone()]),
                                root: path,
                                tree,
                            }),
                            resolved,
                            conda: None,
                        }
                    }
                    AcquisitionSource::Package { manifest, .. } => {
                        super::sharing::acquire(ctx, session, manifest, ctx.fetch.limits())?
                    }
                    AcquisitionSource::CondaEnvironment(_) | AcquisitionSource::PixiLock(_) => {
                        let pin = pin.ok_or_else(|| ExecError::Step {
                            output: (*name).into(),
                            step: "conda".into(),
                            detail:
                                "conda source has no lock; run grip update before frozen acquisition"
                                    .into(),
                        })?;
                        let locked = pin.conda.as_ref().ok_or_else(|| ExecError::Step {
                            output: (*name).into(),
                            step: "conda".into(),
                            detail: "conda pin is missing its frozen closure".into(),
                        })?;
                        super::conda::admit_frozen(name, source, locked)?;
                        if let AcquisitionSource::PixiLock(declared) = source {
                            super::conda::verify_pixi_inputs(
                                ctx, session, workspace, name, declared, pin,
                            )?;
                        }
                        let materialized =
                            super::conda::materialize_locked(ctx, session, name, locked)?;
                        PreparedSource {
                            resolved: pin.resolved.clone(),
                            conda: Some(materialized.receipt.clone()),
                            artifact: Arc::new(materialized.artifact),
                        }
                    }
                };
                sources.insert(*name, prepared);
            }
        }
        let mut needed_inputs = BTreeSet::new();
        for name in &selection.required {
            collect_inputs(selection.outputs[name], &mut needed_inputs);
        }
        let mut inputs = BTreeMap::new();
        let mut command_pins = CommandPins::default();
        for input in &workspace.inputs {
            if needed_inputs.contains(input.name.as_str()) {
                let captured = inputs::capture(ctx, session, input)?;
                command_pins
                    .inputs
                    .insert(input.name.clone(), captured.identity);
                inputs.insert(input.name.as_str(), captured);
            }
        }
        Self::identities(workspace, selection, sources, inputs, command_pins)
    }

    pub(super) fn identities(
        workspace: &'a WorkspaceCatalog,
        selection: Selection<'a>,
        sources: BTreeMap<&'a str, PreparedSource>,
        inputs: BTreeMap<&'a str, CapturedInput>,
        mut command_pins: CommandPins,
    ) -> Result<Self, ExecError> {
        admit_capabilities(&selection)?;
        let mut pending: BTreeSet<_> = selection
            .required
            .iter()
            .copied()
            .filter(|name| matches!(selection.outputs[name], WorkspaceOutput::Package(_)))
            .collect();
        let mut package_productions = BTreeMap::new();
        while !pending.is_empty() {
            let before = pending.len();
            let names: Vec<_> = pending.iter().copied().collect();
            for name in names {
                let result = match selection.outputs[name] {
                    WorkspaceOutput::Package(package) => {
                        let digest = identity::provider_package_production_digest(
                            package,
                            &sources[name].resolved,
                            &package_productions,
                        );
                        match digest {
                            Ok(digest) => {
                                let provided = artifact::inspect_commands(
                                    package,
                                    &sources[name].artifact.payload,
                                )?;
                                let mut tools = BTreeMap::new();
                                for (command, selector) in &package.commands {
                                    let executable =
                                        ToolExecutable::Captured(provided[command].executable);
                                    tools.insert(
                                        command.clone(),
                                        ToolPin {
                                            executable,
                                            package: digest,
                                            selector: selector.clone(),
                                        },
                                    );
                                }
                                command_pins.tools.insert(name.to_owned(), tools);
                                command_pins.artifacts.insert(
                                    name.to_owned(),
                                    ArtifactDigest::package_output(digest),
                                );
                                package_productions.insert(name.to_owned(), digest);
                                Ok(())
                            }
                            Err(error) => Err(error),
                        }
                    }
                    _ => unreachable!("production selection contains only packages"),
                };
                match result {
                    Ok(()) => {
                        pending.remove(name);
                    }
                    Err(PinGap::Artifact(_) | PinGap::Tool { .. }) => {}
                    Err(error) => {
                        return Err(failure(
                            Some(selection.outputs[name].span()),
                            error.to_string(),
                        ));
                    }
                }
            }
            if pending.len() == before {
                return Err(failure(
                    Some(&workspace.span),
                    "production identity dependencies are cyclic or unresolved",
                ));
            }
        }
        let mut packages = BTreeMap::new();
        let mut remaining: BTreeSet<_> = package_productions.keys().map(String::as_str).collect();
        while !remaining.is_empty() {
            let before = remaining.len();
            let names: Vec<_> = remaining.iter().copied().collect();
            for name in names {
                let WorkspaceOutput::Package(package) = selection.outputs[name] else {
                    unreachable!("package identity names a package");
                };
                let digest =
                    identity::provider_package_digest(package, &sources[name].resolved, &packages);
                match digest {
                    Ok(digest) => {
                        packages.insert(name.to_owned(), digest);
                        remaining.remove(name);
                    }
                    Err(PinGap::Artifact(_)) => {}
                    Err(error) => return Err(failure(Some(&package.span), error.to_string())),
                }
            }
            if remaining.len() == before {
                return Err(failure(
                    Some(&workspace.span),
                    "runtime package identities are unresolved",
                ));
            }
        }
        Ok(Self {
            selection,
            sources,
            inputs,
            packages,
        })
    }

    pub fn retained_inputs(&self, ctx: &Ctx, output: &str) -> BTreeSet<PathBuf> {
        self.retained_roots(&ctx.home, output)
    }

    pub(super) fn retained_roots(&self, home: &std::path::Path, output: &str) -> BTreeSet<PathBuf> {
        let mut roots = BTreeSet::new();
        let mut pending = VecDeque::from([output]);
        let mut visited = BTreeSet::new();
        while let Some(name) = pending.pop_front() {
            if !visited.insert(name) {
                continue;
            }
            if let Some(source) = self.sources.get(name) {
                roots.extend(source.artifact.retention.iter().cloned());
            }
            if let Some(package) = self.packages.get(name) {
                roots.insert(gripsack_store::content_path(
                    home,
                    "workspace-package",
                    &package.to_string(),
                ));
            }
            let mut inputs = BTreeSet::new();
            collect_inputs(self.selection.outputs[name], &mut inputs);
            for input in inputs {
                if let Some(input) = self.inputs.get(input) {
                    roots.extend(input.artifact.retention.iter().cloned());
                }
            }
            for edge in self.selection.references(name) {
                pending.push_back(edge.to);
            }
        }
        roots
    }
}

pub(super) fn collect_inputs<'a>(output: &'a WorkspaceOutput, inputs: &mut BTreeSet<&'a str>) {
    let mut command = |command: &'a WorkspaceCommand| {
        for argument in command.arguments() {
            if let WorkspaceArg::Input { input } = argument {
                inputs.insert(input);
            }
        }
    };
    match output {
        WorkspaceOutput::Task(task) => {
            for step in &task.steps {
                if let WorkspaceStep::Command(value) = step {
                    command(value);
                }
            }
        }
        WorkspaceOutput::Check(check) => command(&check.run),
        WorkspaceOutput::Hook(hook) => command(&hook.run),
        WorkspaceOutput::Environment(environment) => {
            for argument in environment.env.values() {
                if let WorkspaceArg::Input { input } = argument {
                    inputs.insert(input);
                }
            }
        }
        _ => {}
    }
}

fn admit_capabilities(selection: &Selection<'_>) -> Result<(), ExecError> {
    for name in &selection.required {
        let output = selection.outputs[name];
        if matches!(
            output,
            WorkspaceOutput::Recipe(_) | WorkspaceOutput::Image(_)
        ) || matches!(
            output,
            WorkspaceOutput::Package(PackageOutput {
                producer: WorkspaceProducer::Recipe { .. },
                ..
            })
        ) {
            return Err(failure(
                Some(output.span()),
                "executable recipe and OCI image authoring have been retired",
            ));
        }
    }
    Ok(())
}
fn failure(span: Option<&Span>, detail: impl Into<String>) -> ExecError {
    ExecError::Gate(
        Diagnostic::error(codes::BAD_WORKSPACE_CONTEXT, detail)
            .with_label(span.cloned(), "workspace production declared here"),
    )
}
