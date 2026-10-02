//! Native preparation and identity calculation for one selected output closure.
//! This is planning, not per-recipe execution. Production identities precede
//! required-check identities so self/mutual validation does not recurse through
//! an unknown output checksum. The finite validation closure then fences reuse.
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
    workspace_v6::{
        identity::{
            self, ArtifactDigest, CheckDigest, CommandPins, PackageDigest, PackageProductionDigest,
            PinGap, RecipeDigest, RecipePins, ToolExecutable, ToolPin,
        },
        lock::{DefinitionPins, ResolvedPinFields},
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
    pub recipes: BTreeMap<&'a str, RecipeDigest>,
    pub packages: BTreeMap<String, PackageDigest>,
    pub package_productions: BTreeMap<String, PackageProductionDigest>,
    pub command_pins: CommandPins,
    pub checks: BTreeMap<(&'a str, &'a str), CheckDigest>,
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
        let workspace = ir
            .workspace_v6
            .as_ref()
            .filter(|_| {
                ir.ir_version == gripsack_ir::IR_VERSION
                    && ir.workspace.is_none()
                    && ir.workspace_v4.is_none()
                    && ir.modules.is_empty()
            })
            .ok_or_else(|| {
                failure(
                    None,
                    "selected-output production requires a current v6 workspace",
                )
            })?;
        let selection = Selection::admit(workspace, requested)?;
        admit_capabilities(&selection)?;
        let definitions = captured_definitions(&ctx.repository)?;
        let pins = WorkspacePins::read(&ctx.repository)?;
        pins.admit_definitions(&definitions)?;
        let mut sources = BTreeMap::new();
        for name in &selection.required {
            let output = selection.outputs[name];
            let source = match output {
                WorkspaceOutput::Recipe(recipe) => Some((&recipe.source, &recipe.target)),
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
                    WorkspaceSourceV6::Fetch(fetch) => {
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
                    WorkspaceSourceV6::CondaEnvironment(_) | WorkspaceSourceV6::PixiLock(_) => {
                        let pin = pin.ok_or_else(|| ExecError::Step {
                            module: (*name).into(),
                            step: "conda".into(),
                            detail:
                                "conda source has no lock; run grip update before frozen acquisition"
                                    .into(),
                        })?;
                        let locked = pin.conda.as_ref().ok_or_else(|| ExecError::Step {
                            module: (*name).into(),
                            step: "conda".into(),
                            detail: "conda pin is missing its frozen closure".into(),
                        })?;
                        super::conda::admit_frozen(name, source, locked)?;
                        if let WorkspaceSourceV6::PixiLock(declared) = source {
                            super::conda::verify_pixi_inputs(
                                ctx, session, workspace, name, declared, pin,
                            )?;
                        }
                        let materialized = super::conda::materialize_locked(
                            ctx,
                            session,
                            name,
                            locked,
                            super::conda::CondaDestination::Native,
                        )?;
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
        Self::identities(
            workspace,
            selection,
            &definitions,
            sources,
            inputs,
            command_pins,
        )
    }

    pub(super) fn identities(
        workspace: &'a WorkspaceV6,
        selection: Selection<'a>,
        definitions: &DefinitionPins,
        sources: BTreeMap<&'a str, PreparedSource>,
        inputs: BTreeMap<&'a str, CapturedInput>,
        mut command_pins: CommandPins,
    ) -> Result<Self, ExecError> {
        let mut pending: BTreeSet<_> = selection
            .required
            .iter()
            .copied()
            .filter(|name| {
                matches!(
                    selection.outputs[name],
                    WorkspaceOutput::Recipe(_) | WorkspaceOutput::Package(_)
                )
            })
            .collect();
        let mut productions = BTreeMap::new();
        let mut package_productions = BTreeMap::new();
        let empty_checks = BTreeMap::new();
        let empty_closure = BTreeSet::new();
        while !pending.is_empty() {
            let before = pending.len();
            let names: Vec<_> = pending.iter().copied().collect();
            for name in names {
                let result = match selection.outputs[name] {
                    WorkspaceOutput::Recipe(recipe) => identity::production_digest(
                        recipe,
                        &RecipePins {
                            definitions,
                            source: &sources[name].resolved,
                            commands: &command_pins,
                            checks: &empty_checks,
                            transitive_checks: &empty_closure,
                        },
                    )
                    .map(|digest| {
                        productions.insert(name, digest);
                        command_pins
                            .artifacts
                            .insert(name.to_owned(), ArtifactDigest::recipe_output(digest));
                    }),
                    WorkspaceOutput::Package(package) => {
                        let digest = match &package.producer {
                            WorkspaceProducer::Recipe { recipe } => {
                                match productions.get(recipe.as_str()) {
                                    Some(recipe) => identity::recipe_package_production_digest(
                                        package,
                                        *recipe,
                                        &package_productions,
                                    ),
                                    None => continue,
                                }
                            }
                            WorkspaceProducer::Provider { .. } => {
                                identity::provider_package_production_digest(
                                    package,
                                    &sources[name].resolved,
                                    &package_productions,
                                )
                            }
                        };
                        match digest {
                            Ok(digest) => {
                                let provided = match &package.producer {
                                    WorkspaceProducer::Provider { .. } => {
                                        Some(artifact::inspect_commands(
                                            package,
                                            &sources[name].artifact.payload,
                                        )?)
                                    }
                                    WorkspaceProducer::Recipe { .. } => None,
                                };
                                let mut tools = BTreeMap::new();
                                for (command, selector) in &package.commands {
                                    let executable = match &package.producer {
                                        WorkspaceProducer::Provider { .. } => {
                                            ToolExecutable::Captured(
                                                provided
                                                    .as_ref()
                                                    .expect("provider commands inspected")[command]
                                                    .executable,
                                            )
                                        }
                                        WorkspaceProducer::Recipe { recipe } => {
                                            ToolExecutable::Produced(productions[recipe.as_str()])
                                        }
                                    };
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
                    _ => unreachable!("production selection contains only recipes/packages"),
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
        let mut checks = BTreeMap::new();
        for &name in productions.keys() {
            let WorkspaceOutput::Recipe(recipe) = selection.outputs[name] else {
                unreachable!("production names a recipe");
            };
            for check_name in &recipe.checks {
                let WorkspaceOutput::Check(check) = selection.outputs[check_name.as_str()] else {
                    unreachable!("check references admitted");
                };
                checks.insert(
                    (name, check_name.as_str()),
                    identity::check_digest(check, &command_pins, &recipe.execution)
                        .map_err(|error| failure(Some(&check.span), error.to_string()))?,
                );
            }
        }
        let mut recipes = BTreeMap::new();
        for &name in productions.keys() {
            let WorkspaceOutput::Recipe(recipe) = selection.outputs[name] else {
                unreachable!("production identity names a recipe");
            };
            let direct_checks = recipe
                .checks
                .iter()
                .map(|check| (check.clone(), checks[&(name, check.as_str())]))
                .collect();
            let required_checks = validation_closure(&selection, name, &checks);
            let validated = identity::recipe_digest(
                recipe,
                &RecipePins {
                    definitions,
                    source: &sources[name].resolved,
                    commands: &command_pins,
                    checks: &direct_checks,
                    transitive_checks: &required_checks,
                },
            )
            .map_err(|error| failure(Some(&recipe.span), error.to_string()))?;
            recipes.insert(name, validated);
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
                let digest = match &package.producer {
                    WorkspaceProducer::Recipe { recipe } => identity::recipe_package_digest(
                        package,
                        recipes[recipe.as_str()],
                        &packages,
                    ),
                    WorkspaceProducer::Provider { .. } => identity::provider_package_digest(
                        package,
                        &sources[name].resolved,
                        &packages,
                    ),
                };
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
            recipes,
            packages,
            package_productions,
            command_pins,
            checks,
        })
    }

    pub fn retained_inputs(&self, ctx: &Ctx, recipe: &str) -> BTreeSet<PathBuf> {
        self.retained_roots(&ctx.home, recipe)
    }

    pub(super) fn retained_roots(&self, home: &std::path::Path, recipe: &str) -> BTreeSet<PathBuf> {
        let mut roots = BTreeSet::new();
        let mut pending = VecDeque::from([recipe]);
        let mut visited = BTreeSet::new();
        while let Some(name) = pending.pop_front() {
            if !visited.insert(name) {
                continue;
            }
            if let Some(source) = self.sources.get(name) {
                roots.extend(source.artifact.retention.iter().cloned());
            }
            if name != recipe {
                if let Some(recipe) = self.recipes.get(name) {
                    roots.insert(gripsack_store::content_path(
                        home,
                        "workspace-recipe",
                        &recipe.to_string(),
                    ));
                }
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

fn validation_closure(
    selection: &Selection<'_>,
    root: &str,
    checks: &BTreeMap<(&str, &str), CheckDigest>,
) -> BTreeSet<CheckDigest> {
    let mut result = BTreeSet::new();
    let mut pending = VecDeque::from([root]);
    let mut visited = BTreeSet::new();
    while let Some(name) = pending.pop_front() {
        if !visited.insert(name) {
            continue;
        }
        if let WorkspaceOutput::Recipe(recipe) = selection.outputs[name] {
            for check in &recipe.checks {
                result.insert(checks[&(name, check.as_str())]);
            }
        }
        for edge in selection.references(name) {
            pending.push_back(edge.to);
        }
    }
    result
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
        WorkspaceOutput::Recipe(recipe) => {
            for step in &recipe.steps {
                if let WorkspaceStep::Command(value) = step {
                    command(value);
                }
            }
        }
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
        if let WorkspaceOutput::Recipe(recipe) = output {
            match &recipe.execution {
                RecipeExecution::Host { .. } => return Err(ExecError::Gate(Diagnostic::error(codes::WORKSPACE_EXEC_UNAVAILABLE, "host recipe realization belongs to its native A2 executor; it cannot enter an isolated BuildKit solve").with_label(Some(recipe.span.clone()), "host policy declared here"))),
                RecipeExecution::IsolatedLinux { platform, .. } if platform.os != recipe.target.os || platform.arch != recipe.target.arch => return Err(failure(Some(&recipe.span), "cross-compilation is not in the admitted initial BuildKit subset")),
                RecipeExecution::IsolatedLinux { .. } => {},
            }
            for check in &recipe.checks {
                let WorkspaceOutput::Check(check) = selection.outputs[check.as_str()] else {
                    return Err(failure(
                        Some(&recipe.span),
                        "required validator is not a check",
                    ));
                };
                if !matches!(
                    selection.outputs[check.subject.as_str()],
                    WorkspaceOutput::Recipe(_) | WorkspaceOutput::Package(_)
                ) {
                    return Err(failure(
                        Some(&check.span),
                        "a production validator requires a recipe/package subject",
                    ));
                }
            }
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
