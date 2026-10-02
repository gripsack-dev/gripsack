//! Native acquire/retain -> one compatible solve -> validate all -> publish.
//! No per-recipe process scheduler and no personal generation are involved.
use super::{
    artifact::{self, Artifact, Package, ProducerIdentity},
    lowering,
    prepare::Prepared,
    roots::{self, RetentionSet, RootId},
    stage,
};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_buildkit::worker::WorkerOptions;
use gripsack_ir::{
    Ir,
    workspace_v6::{
        WorkspaceArg, WorkspaceCommand, WorkspaceOutput, WorkspaceProducer, WorkspaceStep,
        identity::ExecutableDigest,
    },
};
use gripsack_process::OperatorEnvironment;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

pub struct BuildOptions<'a> {
    pub environment: &'a OperatorEnvironment,
    pub bridge: Option<&'a Path>,
    pub worker: WorkerOptions,
    pub deadline: Instant,
}
#[derive(Serialize)]
pub struct BuiltOutput {
    pub name: String,
    pub kind: &'static str,
    pub path: PathBuf,
    pub commands: BTreeMap<String, PathBuf>,
}
#[derive(Serialize)]
pub struct BuildResult {
    pub outputs: Vec<BuiltOutput>,
}

pub(crate) struct Realization<'a> {
    pub(super) recipes: BTreeMap<&'a str, Arc<Artifact>>,
    pub(super) packages: BTreeMap<&'a str, Arc<Package>>,
    /// Captured workspace input binding paths (file inputs carry their
    /// `value` member); empty when the selection declares no inputs.
    pub(super) inputs: BTreeMap<&'a str, PathBuf>,
}

pub fn build_workspace(
    ir: &Ir,
    ctx: &Ctx,
    options: &BuildOptions<'_>,
) -> Result<BuildResult, ExecError> {
    let workspace = ir
        .workspace_v6
        .as_ref()
        .ok_or_else(|| failure("build requires a current typed workspace"))?;
    let selection = super::selection::Selection::admit(workspace, &ctx.only)?;
    for name in &ctx.only {
        if !matches!(
            selection.outputs[name.as_str()],
            WorkspaceOutput::Recipe(_) | WorkspaceOutput::Package(_) | WorkspaceOutput::Image(_)
        ) {
            return Err(failure(format!(
                "{name:?} is not a recipe, package or image output"
            )));
        }
    }
    let HeldRealization {
        realization: realized,
        mut session,
        closures,
        build,
    } = realize_held(ir, ctx, options, &ctx.only)?;
    let has_images = ctx
        .only
        .iter()
        .any(|name| matches!(selection.outputs[name.as_str()], WorkspaceOutput::Image(_)));
    let mut in_flight = BTreeSet::new();
    for (name, paths) in closures {
        if has_images {
            in_flight.extend(paths.iter().cloned());
        }
        if !matches!(selection.outputs[name.as_str()], WorkspaceOutput::Image(_)) {
            let id =
                RootId::from_identity(&serde_json::to_vec(&(ctx.repository.identity(), &name))?);
            let consumer = RetentionSet::admit(&session, paths)?;
            roots::register_output_root(&session, &id, &consumer)?;
        }
    }
    let mut outputs = Vec::with_capacity(ctx.only.len());
    for name in &ctx.only {
        let output = match selection.outputs[name.as_str()] {
            WorkspaceOutput::Recipe(_) => BuiltOutput {
                name: name.clone(),
                kind: "recipe",
                path: realized.recipes[name.as_str()].payload.clone(),
                commands: BTreeMap::new(),
            },
            WorkspaceOutput::Package(_) => {
                let package = &realized.packages[name.as_str()];
                BuiltOutput {
                    name: name.clone(),
                    kind: "package",
                    path: package.producer.payload.clone(),
                    commands: package
                        .commands
                        .keys()
                        .map(|command| {
                            (
                                command.clone(),
                                package.command(command).expect("provided command"),
                            )
                        })
                        .collect(),
                }
            }
            WorkspaceOutput::Image(image) => {
                let (next_session, output) = super::image::export(
                    ctx, options, image, &selection, &realized, &in_flight, session,
                )?;
                session = next_session;
                output
            }
            _ => unreachable!("build consumer admitted"),
        };
        outputs.push(output);
    }
    if let Some(completed) = build {
        completed.finish(&session)?;
    }
    Ok(BuildResult { outputs })
}

fn cached_recipes<'a>(
    prepared: &Prepared<'a>,
    ctx: &Ctx,
    session: &LifecycleSession,
) -> Result<BTreeMap<&'a str, Arc<Artifact>>, ExecError> {
    let mut recipes = BTreeMap::new();
    for (&name, identity) in &prepared.recipes {
        let WorkspaceOutput::Recipe(recipe) = prepared.selection.outputs[name] else {
            unreachable!();
        };
        let inputs = prepared.retained_inputs(ctx, name);
        if let Some(artifact) = artifact::cached_recipe(
            ctx,
            session,
            *identity,
            recipe.output_kind,
            &recipe.execution,
            &inputs,
        )? {
            recipes.insert(name, artifact);
        }
    }
    Ok(recipes)
}

fn input_paths<'a>(prepared: &Prepared<'a>) -> BTreeMap<&'a str, PathBuf> {
    prepared
        .inputs
        .iter()
        .map(|(name, input)| (*name, input.binding_path()))
        .collect()
}

/// A realization that still holds its lifecycle session. Callers register
/// their own consumer roots (project selections, process leases, generation
/// closures) atomically with the realization — there is no GC window between
/// publication and root acquisition. `build_workspace` keeps per-output
/// roots; native consumers register project/process roots instead.
pub(super) struct HeldRealization<'a> {
    pub realization: Realization<'a>,
    pub session: LifecycleSession,
    /// Exact realized closure per selected output, already proven to be a
    /// subset of the protected build retention.
    pub closures: Vec<(String, BTreeSet<PathBuf>)>,
    build: Option<super::solve::CompletedSolve>,
}
impl HeldRealization<'_> {
    /// Retire the completed build's root and staging after the caller has
    /// registered its own roots. A no-op on the no-build path.
    pub fn finish_build(&mut self) -> Result<(), ExecError> {
        if let Some(completed) = self.build.take() {
            completed.finish(&self.session)?;
        }
        Ok(())
    }
}

pub(super) fn realize_held<'a>(
    ir: &'a Ir,
    ctx: &Ctx,
    options: &BuildOptions<'_>,
    selected: &[String],
) -> Result<HeldRealization<'a>, ExecError> {
    let session = LifecycleSession::acquire(&ctx.home)?;
    let prepared = Prepared::frozen(ir, ctx, &session, selected)?;
    let mut retention = BTreeSet::new();
    for source in prepared.sources.values() {
        retention.extend(source.artifact.retention.iter().cloned());
    }
    for input in prepared.inputs.values() {
        retention.extend(input.artifact.retention.iter().cloned());
    }
    for identity in prepared.recipes.values() {
        retention.insert(gripsack_store::content_path(
            &ctx.home,
            "workspace-recipe",
            &identity.to_string(),
        ));
    }
    let mut recipes = cached_recipes(&prepared, ctx, &session)?;
    for &name in prepared.recipes.keys() {
        retention.extend(prepared.retained_inputs(ctx, name).iter().cloned());
    }
    for identity in prepared.packages.values() {
        retention.insert(gripsack_store::content_path(
            &ctx.home,
            "workspace-package",
            &identity.to_string(),
        ));
    }
    let retention = RetentionSet::admit(&session, retention)?;
    let projection = lowering::compile(&prepared, &recipes)?;
    let mut completed_build = None;
    let session = if let Some(projection) = projection {
        let lowering::Projection {
            plan,
            sources,
            outputs,
            mut origins,
        } = projection;
        let (session, completed) = super::solve::execute(
            ctx,
            options,
            &plan,
            &sources,
            &mut origins,
            &retention,
            session,
        )?;
        drop(sources);
        let output = completed.output();
        // Validate the aggregate as well: exporter debris cannot evade limits by
        // living outside the named candidate directories.
        gripsack_fetch::fetch::validate_tree(&output, ctx.fetch.limits())?;
        let mut candidates = BTreeMap::new();
        for (name, relative) in outputs {
            let WorkspaceOutput::Recipe(recipe) = prepared.selection.outputs[name] else {
                unreachable!();
            };
            candidates.insert(
                name,
                stage::validate_output_tree(
                    &output.join(relative),
                    recipe.output_kind,
                    name,
                    ctx.fetch.limits(),
                )?,
            );
        }
        validate_commands(&prepared, &recipes, &candidates)?;
        for (name, candidate) in candidates {
            let WorkspaceOutput::Recipe(recipe) = prepared.selection.outputs[name] else {
                unreachable!();
            };
            let artifact = artifact::publish_recipe(
                ctx,
                &session,
                prepared.recipes[name],
                recipe.output_kind,
                &recipe.execution,
                candidate,
                prepared.retained_inputs(ctx, name),
            )?;
            recipes.insert(name, artifact);
        }
        completed_build = Some(completed);
        session
    } else {
        session
    };
    let packages = publish_packages(&prepared, &recipes, ctx, &session)?;
    let mut closures = Vec::with_capacity(selected.len());
    for name in selected {
        let paths = if let Some(recipe) = recipes.get(name.as_str()) {
            recipe.retention.clone()
        } else if let Some(package) = packages.get(name.as_str()) {
            let mut paths = BTreeSet::new();
            package.retain_into(&mut paths);
            paths
        } else {
            prepared.retained_inputs(ctx, name)
        };
        if !paths.is_subset(retention.paths()) {
            return Err(failure(
                "realized consumer closure contains an object outside its protected build roots",
            ));
        }
        closures.push((name.clone(), paths));
    }
    Ok(HeldRealization {
        realization: Realization {
            recipes,
            packages,
            inputs: input_paths(&prepared),
        },
        session,
        closures,
        build: completed_build,
    })
}

fn validate_commands(
    prepared: &Prepared<'_>,
    cached: &BTreeMap<&str, Arc<Artifact>>,
    candidates: &BTreeMap<&str, stage::ValidatedTree>,
) -> Result<(), ExecError> {
    let mut provided = BTreeMap::new();
    for name in prepared.packages.keys() {
        let WorkspaceOutput::Package(package) = prepared.selection.outputs[name.as_str()] else {
            unreachable!();
        };
        let payload = match &package.producer {
            WorkspaceProducer::Provider { .. } => &prepared.sources[name.as_str()].artifact.payload,
            WorkspaceProducer::Recipe { recipe } => {
                if let Some(candidate) = candidates.get(recipe.as_str()) {
                    candidate.root()
                } else {
                    &cached[recipe.as_str()].payload
                }
            }
        };
        provided.insert(name.as_str(), artifact::inspect_commands(package, payload)?);
    }
    let inspect = |command: &WorkspaceCommand| -> Result<(), ExecError> {
        for argument in command.arguments() {
            if let WorkspaceArg::PackageCommand {
                package,
                command,
                sha256: Some(claim),
            } = argument
            {
                if ExecutableDigest::parse(claim).map_err(operational)?
                    != provided[package.as_str()][command].executable
                {
                    return Err(failure(format!(
                        "executable claim for {package:?}/{command:?} differs from the produced bytes"
                    )));
                }
            }
        }
        Ok(())
    };
    for name in &prepared.selection.required {
        match prepared.selection.outputs[name] {
            WorkspaceOutput::Recipe(recipe) => {
                for step in &recipe.steps {
                    if let WorkspaceStep::Command(command) = step {
                        inspect(command)?;
                    }
                }
            }
            WorkspaceOutput::Check(check) => inspect(&check.run)?,
            _ => {}
        }
    }
    Ok(())
}
fn publish_packages<'a>(
    prepared: &Prepared<'a>,
    recipes: &BTreeMap<&str, Arc<Artifact>>,
    ctx: &Ctx,
    session: &LifecycleSession,
) -> Result<BTreeMap<&'a str, Arc<Package>>, ExecError> {
    let mut packages = BTreeMap::new();
    let mut pending: BTreeSet<_> = prepared
        .selection
        .required
        .iter()
        .copied()
        .filter(|name| {
            matches!(
                prepared.selection.outputs[name],
                WorkspaceOutput::Package(_)
            )
        })
        .collect();
    while !pending.is_empty() {
        let before = pending.len();
        for name in pending.iter().copied().collect::<Vec<_>>() {
            let WorkspaceOutput::Package(package) = prepared.selection.outputs[name] else {
                unreachable!();
            };
            if package
                .runtime
                .iter()
                .any(|runtime| !packages.contains_key(runtime.as_str()))
            {
                continue;
            }
            let (producer, identity) = match &package.producer {
                WorkspaceProducer::Provider { .. } => (
                    Arc::clone(&prepared.sources[name].artifact),
                    ProducerIdentity::Provider(&prepared.sources[name].resolved),
                ),
                WorkspaceProducer::Recipe { recipe } => (
                    Arc::clone(&recipes[recipe.as_str()]),
                    ProducerIdentity::Recipe(prepared.recipes[recipe.as_str()]),
                ),
            };
            let runtime = package
                .runtime
                .iter()
                .map(|name| Arc::clone(&packages[name.as_str()]))
                .collect();
            packages.insert(
                name,
                artifact::publish_package(
                    ctx,
                    session,
                    package,
                    producer,
                    identity,
                    runtime,
                    &prepared.packages,
                    match &package.producer {
                        WorkspaceProducer::Provider { .. } => prepared.sources[name].conda.clone(),
                        WorkspaceProducer::Recipe { .. } => None,
                    },
                )?,
            );
            pending.remove(name);
        }
        if pending.len() == before {
            return Err(failure("runtime package closure cannot be published"));
        }
    }
    Ok(packages)
}
fn operational(error: impl std::fmt::Display) -> ExecError {
    failure(error.to_string())
}
fn failure(detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        module: "workspace".into(),
        step: "build".into(),
        detail: detail.into(),
    }
}
