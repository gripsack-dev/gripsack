//! One admitted workspace closure becomes one backend solve. These traversals
//! emit a graph; they never execute or schedule individual recipes/commands.
mod commands;
mod provenance;
pub(super) use provenance::{Origin, SourceMap};

use super::{artifact::Artifact, prepare::Prepared};
use crate::ExecError;
use gripsack_buildkit::{
    identity::SnapshotDigest,
    plan::{
        Architecture, BuildPlan, ExporterPlan, LinuxOs, Mount, Node, NodeIndex, Platform,
        ValidatedBuildPlan,
    },
};
use gripsack_ir::{
    Diagnostic, Span, codes,
    workspace::{PlatformArch, PlatformOs},
    workspace_v6::{
        RecipeExecution, WorkspaceAction, WorkspaceOutput, WorkspaceProducer, WorkspaceStep,
        identity::CheckDigest,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(super) struct Projection<'p, 'ir> {
    pub plan: ValidatedBuildPlan,
    pub sources: BTreeMap<String, &'p Artifact>,
    /// All newly produced artifacts, including producers used by other outputs.
    pub outputs: BTreeMap<&'ir str, String>,
    pub origins: SourceMap<'ir>,
}

struct Compiler<'p, 'ir> {
    prepared: &'p Prepared<'ir>,
    cached: &'p BTreeMap<&'ir str, Arc<Artifact>>,
    nodes: Vec<Node>,
    sources: BTreeMap<String, &'p Artifact>,
    locals: BTreeMap<String, NodeIndex>,
    images: BTreeMap<&'ir str, NodeIndex>,
    recipes: BTreeMap<&'ir str, NodeIndex>,
    visiting: BTreeSet<&'ir str>,
    origin: Origin<'ir>,
    origins: SourceMap<'ir>,
}

pub(super) fn compile<'p, 'ir>(
    prepared: &'p Prepared<'ir>,
    cached: &'p BTreeMap<&'ir str, Arc<Artifact>>,
) -> Result<Option<Projection<'p, 'ir>>, ExecError> {
    let missing: Vec<_> = prepared
        .recipes
        .keys()
        .copied()
        .filter(|name| !cached.contains_key(name))
        .collect();
    let Some(&first) = missing.first() else {
        return Ok(None);
    };
    let first_output = prepared.selection.outputs[first];
    let WorkspaceOutput::Recipe(first_recipe) = first_output else {
        unreachable!("recipe identity names a recipe");
    };
    let RecipeExecution::IsolatedLinux { platform, .. } = &first_recipe.execution else {
        return Err(failure(
            &first_recipe.span,
            "host production cannot enter BuildKit",
        ));
    };
    let platform = platform_for(platform, &first_recipe.span)?;
    let mut compiler = Compiler {
        prepared,
        cached,
        nodes: Vec::new(),
        sources: BTreeMap::new(),
        locals: BTreeMap::new(),
        images: BTreeMap::new(),
        recipes: BTreeMap::new(),
        visiting: BTreeSet::new(),
        origin: Origin {
            output: "workspace",
            span: &prepared.selection.workspace.span,
            line_map: &[],
        },
        origins: SourceMap::default(),
    };
    // Capability admission precedes lowering, bridge provisioning and execution.
    for &name in &missing {
        let WorkspaceOutput::Recipe(recipe) = prepared.selection.outputs[name] else {
            unreachable!();
        };
        let RecipeExecution::IsolatedLinux {
            platform: required, ..
        } = &recipe.execution
        else {
            return Err(failure(
                &recipe.span,
                "mixed host/isolated production requires distinct native execution authority",
            ));
        };
        if platform_for(required, &recipe.span)? != platform {
            return Err(failure(
                &recipe.span,
                "selected producers require incompatible worker platforms",
            ));
        }
    }
    for &name in &missing {
        compiler.origin = Origin {
            output: name,
            span: prepared.selection.outputs[name].span(),
            line_map: &[],
        };
        compiler.recipe(name)?;
    }
    // Export every candidate in the same solve. No candidate is publishable until
    // all required checks complete and the native exporter validation succeeds.
    let mut root = None;
    let mut outputs = BTreeMap::new();
    for &name in &missing {
        compiler.origin = Origin {
            output: name,
            span: prepared.selection.outputs[name].span(),
            line_map: &[],
        };
        let path = format!("artifacts/{}", prepared.recipes[name]);
        root = Some(compiler.push(Node::Copy {
            input: root,
            source: compiler.recipes[name],
            source_path: "/".into(),
            destination: format!("/{path}"),
            contents: true,
        })?);
        outputs.insert(name, path);
    }
    let mut validated = BTreeMap::<CheckDigest, NodeIndex>::new();
    for (&(owner, check_name), &digest) in &prepared.checks {
        if !missing.contains(&owner) {
            continue;
        }
        let WorkspaceOutput::Recipe(recipe) = prepared.selection.outputs[owner] else {
            unreachable!();
        };
        let WorkspaceOutput::Check(check) = prepared.selection.outputs[check_name] else {
            unreachable!();
        };
        let requester = Origin {
            output: owner,
            span: &recipe.span,
            line_map: &[],
        };
        let check_origin = Origin {
            output: check_name,
            span: check.run.span(),
            line_map: match &check.run {
                gripsack_ir::workspace_v6::WorkspaceCommand::RunBash { line_map, .. } => line_map,
                _ => &[],
            },
        };
        if let Some(&checked) = validated.get(&digest) {
            compiler.origins.note(checked, check_origin);
            compiler.origins.note(checked, requester);
            continue;
        }
        compiler.origin = check_origin;
        // For a validator, sourcePath denotes its immutable subject. The check
        // identity binds that subject and the inherited execution policy.
        let subject = compiler.artifact(&check.subject)?;
        let image = compiler.image(&recipe.execution)?;
        let checked =
            compiler.command(&check.run, image, subject, None, &[check.subject.as_str()])?;
        compiler.origins.note(checked, requester);
        validated.insert(digest, checked);
        let marker = compiler.push(Node::File {
            input: Some(checked),
            path: "/.gripsack-check-passed".into(),
            data: digest.to_string().into_bytes(),
            mode: 0o444,
        })?;
        root = Some(compiler.push(Node::Copy {
            input: root,
            source: marker,
            source_path: "/.gripsack-check-passed".into(),
            destination: format!("/checks/{digest}"),
            contents: false,
        })?);
    }
    let plan = ValidatedBuildPlan::admit(BuildPlan {
        platform,
        nodes: compiler.nodes,
        root: root.expect("missing producer creates an export"),
        exporter: ExporterPlan::Local,
    })
    .map_err(|error| failure(first_output.span(), error.to_string()))?;
    Ok(Some(Projection {
        plan,
        sources: compiler.sources,
        outputs,
        origins: compiler.origins,
    }))
}

impl<'p, 'ir> Compiler<'p, 'ir> {
    fn push(&mut self, node: Node) -> Result<NodeIndex, ExecError> {
        let index = NodeIndex::new(self.nodes.len())
            .map_err(|error| failure(&self.prepared.selection.workspace.span, error.to_string()))?;
        self.nodes.push(node);
        self.origins.push(self.origin);
        Ok(index)
    }
    fn local(&mut self, artifact: &'p Artifact) -> Result<NodeIndex, ExecError> {
        let name = artifact.tree.as_str().to_owned();
        if let Some(&node) = self.locals.get(&name) {
            self.origins.note(node, self.origin);
            return Ok(node);
        }
        let digest = SnapshotDigest::parse(artifact.tree.as_str())
            .map_err(|error| failure(&self.prepared.selection.workspace.span, error.to_string()))?;
        let node = self.push(Node::Local {
            name: name.clone(),
            digest,
        })?;
        self.locals.insert(name.clone(), node);
        self.sources.insert(name, artifact);
        Ok(node)
    }
    fn image(&mut self, execution: &'ir RecipeExecution) -> Result<NodeIndex, ExecError> {
        let RecipeExecution::IsolatedLinux { toolchain, .. } = execution else {
            return Err(failure(
                &self.prepared.selection.workspace.span,
                "a host execution policy cannot supply a worker root",
            ));
        };
        if let Some(&node) = self.images.get(toolchain.reference.as_str()) {
            self.origins.note(node, self.origin);
            return Ok(node);
        }
        let node = self.push(Node::Image {
            reference: toolchain.reference.clone(),
        })?;
        self.images.insert(toolchain.reference.as_str(), node);
        Ok(node)
    }
    fn artifact(&mut self, name: &str) -> Result<NodeIndex, ExecError> {
        let output = self.prepared.selection.outputs.get(name).ok_or_else(|| {
            failure(
                &self.prepared.selection.workspace.span,
                format!("missing admitted artifact {name:?}"),
            )
        })?;
        match output {
            WorkspaceOutput::Recipe(recipe) => self.recipe(&recipe.name),
            WorkspaceOutput::Package(package) => match &package.producer {
                WorkspaceProducer::Recipe { recipe } => self.recipe(recipe),
                WorkspaceProducer::Provider { .. } => {
                    self.local(&self.prepared.sources[package.name.as_str()].artifact)
                }
            },
            _ => Err(failure(
                output.span(),
                "only recipe/package values have production payloads",
            )),
        }
    }
    fn recipe(&mut self, name: &'ir str) -> Result<NodeIndex, ExecError> {
        if let Some(&node) = self.recipes.get(name) {
            self.origins.note(node, self.origin);
            return Ok(node);
        }
        if let Some(artifact) = self.cached.get(name) {
            let node = self.local(artifact)?;
            self.recipes.insert(name, node);
            return Ok(node);
        }
        let WorkspaceOutput::Recipe(recipe) = self.prepared.selection.outputs[name] else {
            unreachable!();
        };
        if !self.visiting.insert(name) {
            return Err(failure(
                &recipe.span,
                "production cycle reached the backend projection",
            ));
        }
        let requester = self.origin;
        self.origin = Origin {
            output: name,
            span: recipe.source.span(),
            line_map: &[],
        };
        let source = self.local(&self.prepared.sources[name].artifact)?;
        let mut output = None;
        let mut prerequisites = Vec::new();
        for step in &recipe.steps {
            match step {
                WorkspaceStep::Action(WorkspaceAction::EnsureArtifact { output, .. }) => {
                    prerequisites.push(output.as_str())
                }
                WorkspaceStep::Command(command) => {
                    self.origin = Origin {
                        output: name,
                        span: command.span(),
                        line_map: &[],
                    };
                    let image = self.image(&recipe.execution)?;
                    output = Some(self.command(command, image, source, output, &prerequisites)?);
                    prerequisites.clear();
                }
            }
        }
        // An acquisition-only recipe retains the verified source. Actual build
        // commands start with an empty output and a separate readonly source.
        let output = match output {
            Some(output) => output,
            None => source,
        };
        self.visiting.remove(name);
        self.recipes.insert(name, output);
        self.origin = requester;
        self.origins.note(output, requester);
        Ok(output)
    }
}
fn platform_for(
    platform: &gripsack_ir::workspace::WorkspacePlatform,
    span: &Span,
) -> Result<Platform, ExecError> {
    if platform.os != PlatformOs::Linux {
        return Err(failure(span, "BuildKit production requires Linux"));
    }
    Ok(Platform {
        os: LinuxOs::Linux,
        architecture: match platform.arch {
            PlatformArch::X86_64 => Architecture::Amd64,
            PlatformArch::Aarch64 => Architecture::Arm64,
        },
    })
}
fn failure(span: &Span, detail: impl Into<String>) -> ExecError {
    ExecError::Gate(
        Diagnostic::error(codes::BAD_WORKSPACE_CONTEXT, detail)
            .with_label(Some(span.clone()), "production projection declared here"),
    )
}
