//! One declared-reference projection shared by admission and realization:
//! `references` walks the actual serde datatypes, in declaration order, and
//! yields every reference an output makes to another output — owner, target
//! name, accepted kinds, source span, role, command, selector and target
//! platform. Captured inputs occupy their own namespace (`input`).
//!
//! Image destinations are placement metadata for already-selected runtime
//! packages, not additional dependencies (checked by image selection).
//! Mutation-lock keys, input origins, registry/toolchain strings and the
//! Conda package/channel selectors likewise do not name workspace outputs.
use super::*;
use crate::Span;
use gripsack_policy::graph::GraphRole;
use std::collections::BTreeMap;
mod kinds;
use kinds::*;
pub mod binding;
pub mod input;
pub use binding::{bind_reference, output_name};
#[derive(Debug, Clone, Copy)]
pub struct Reference<'a> {
    pub from: &'a WorkspaceOutput,
    pub to: &'a str,
    pub kinds: &'static [&'static str],
    pub at: &'a Span,
    pub role: GraphRole,
    pub command: Option<&'a str>,
    pub selector: Option<&'a str>,
    pub target: Option<&'a WorkspacePlatform>,
}
pub fn references(workspace: &WorkspaceCatalog) -> Vec<Reference<'_>> {
    let mut edges = Vec::new();
    let mut i = 0;
    while i < workspace.outputs.len() {
        Collector {
            output: &workspace.outputs[i],
        }
        .collect(&mut edges);
        i += 1;
    }
    edges
}

struct Collector<'a> {
    output: &'a WorkspaceOutput,
}
impl<'a> Collector<'a> {
    fn edge(
        &self,
        edges: &mut Vec<Reference<'a>>,
        to: &'a str,
        kinds: &'static [&'static str],
        at: &'a Span,
        role: GraphRole,
        target: Option<&'a WorkspacePlatform>,
    ) {
        edges.push(Reference {
            from: self.output,
            to,
            kinds,
            at,
            role,
            command: None,
            selector: None,
            target,
        });
    }
    fn named(
        &self,
        edges: &mut Vec<Reference<'a>>,
        names: &'a [String],
        kinds: &'static [&'static str],
        at: &'a Span,
        role: GraphRole,
        target: Option<&'a WorkspacePlatform>,
    ) {
        let mut i = 0;
        while i < names.len() {
            self.edge(edges, &names[i], kinds, at, role, target);
            i += 1;
        }
    }
    fn argument(
        &self,
        edges: &mut Vec<Reference<'a>>,
        value: &'a WorkspaceArg,
        at: &'a Span,
        role: GraphRole,
        target: Option<&'a WorkspacePlatform>,
    ) {
        match value {
            WorkspaceArg::Artifact { output, selector } => {
                let edge = Reference {
                    from: self.output,
                    to: output,
                    kinds: &["recipe", "package"],
                    at,
                    role,
                    command: None,
                    selector: Some(selector),
                    target: None,
                };
                edges.push(edge);
            }
            WorkspaceArg::PackageCommand {
                package, command, ..
            } => {
                let edge = Reference {
                    from: self.output,
                    to: package,
                    kinds: &["package"],
                    at,
                    role,
                    command: Some(command),
                    selector: None,
                    target,
                };
                edges.push(edge);
            }
            WorkspaceArg::Literal { .. }
            | WorkspaceArg::Input { .. }
            | WorkspaceArg::Source { .. }
            | WorkspaceArg::Output { .. } => {}
        }
    }
    fn arguments(
        &self,
        edges: &mut Vec<Reference<'a>>,
        args: &'a [WorkspaceArg],
        at: &'a Span,
        role: GraphRole,
        target: Option<&'a WorkspacePlatform>,
    ) {
        let mut i = 0;
        while i < args.len() {
            self.argument(edges, &args[i], at, role, target);
            i += 1;
        }
    }
    fn environment(
        &self,
        edges: &mut Vec<Reference<'a>>,
        env: &'a BTreeMap<String, WorkspaceArg>,
        at: &'a Span,
        role: GraphRole,
        target: Option<&'a WorkspacePlatform>,
    ) {
        for argument in env.values() {
            self.argument(edges, argument, at, role, target);
        }
    }
    fn command(
        &self,
        edges: &mut Vec<Reference<'a>>,
        value: &'a WorkspaceCommand,
        role: GraphRole,
        target: Option<&'a WorkspacePlatform>,
    ) {
        let (env, cwd, at) = match value {
            WorkspaceCommand::Exec {
                argv,
                env,
                cwd,
                span,
            } => {
                self.arguments(edges, argv, span, role, target);
                (env, cwd, span)
            }
            WorkspaceCommand::RunBash {
                interpreter,
                env,
                cwd,
                span,
                ..
            } => {
                self.argument(edges, interpreter, span, role, target);
                (env, cwd, span)
            }
        };
        self.environment(edges, env, at, role, target);
        if let Some(WorkspacePath::Artifact { output, selector }) = cwd {
            edges.push(Reference {
                from: self.output,
                to: output,
                kinds: kind_names(Kinds::Artifact),
                at,
                role,
                command: None,
                selector: Some(selector),
                target: None,
            });
        }
    }
    fn steps(
        &self,
        edges: &mut Vec<Reference<'a>>,
        values: &'a [WorkspaceStep],
        role: GraphRole,
        target: Option<&'a WorkspacePlatform>,
    ) {
        let mut i = 0;
        while i < values.len() {
            match &values[i] {
                WorkspaceStep::Command(command) => self.command(edges, command, role, target),
                WorkspaceStep::Action(WorkspaceAction::EnsureArtifact { output, span }) => {
                    self.edge(edges, output, kind_names(Kinds::Artifact), span, role, None)
                }
            }
            i += 1;
        }
    }
    fn file(&self, edges: &mut Vec<Reference<'a>>, file: &'a WorkspaceFile) {
        match &file.source {
            Some(WorkspaceSource::ArtifactFile { output, selector }) => edges.push(Reference {
                from: self.output,
                to: output,
                kinds: kind_names(Kinds::Artifact),
                at: &file.span,
                role: GraphRole::Runtime,
                command: None,
                selector: Some(selector),
                target: None,
            }),
            Some(WorkspaceSource::Tree { output, .. }) => self.edge(
                edges,
                output,
                kind_names(Kinds::Artifact),
                &file.span,
                GraphRole::Runtime,
                None,
            ),
            Some(WorkspaceSource::RepoFile { .. } | WorkspaceSource::RepoTree { .. }) | None => {}
        }
        let mut i = 0;
        while i < file.checks.len() {
            let check = &file.checks[i];
            self.edge(
                edges,
                &check.check,
                kind_names(Kinds::Check),
                &check.span,
                GraphRole::Validation,
                None,
            );
            i += 1;
        }
    }
    fn collect(&self, edges: &mut Vec<Reference<'a>>) {
        match self.output {
            WorkspaceOutput::Recipe(value) => {
                let execution = match &value.execution {
                    RecipeExecution::IsolatedLinux { platform, .. } => platform,
                    RecipeExecution::Host { .. } => &value.target,
                };
                self.steps(edges, &value.steps, GraphRole::BuildInput, Some(execution));
                self.named(
                    edges,
                    &value.checks,
                    kind_names(Kinds::Check),
                    &value.span,
                    GraphRole::Validation,
                    None,
                );
            }
            WorkspaceOutput::Package(value) => {
                if let WorkspaceProducer::Recipe { recipe } = &value.producer {
                    self.edge(
                        edges,
                        recipe,
                        kind_names(Kinds::Recipe),
                        &value.span,
                        GraphRole::Production,
                        Some(&value.target),
                    );
                }
                self.named(
                    edges,
                    &value.runtime,
                    kind_names(Kinds::Package),
                    &value.span,
                    GraphRole::Runtime,
                    Some(&value.target),
                );
            }
            WorkspaceOutput::Environment(value) => {
                self.named(
                    edges,
                    &value.packages,
                    kind_names(Kinds::Package),
                    &value.span,
                    GraphRole::Runtime,
                    Some(&value.target),
                );
                self.environment(
                    edges,
                    &value.env,
                    &value.span,
                    GraphRole::Runtime,
                    Some(&value.target),
                );
            }
            WorkspaceOutput::Task(value) => {
                self.steps(edges, &value.steps, GraphRole::Runtime, None);
                self.named(
                    edges,
                    &value.deps,
                    kind_names(Kinds::Task),
                    &value.span,
                    GraphRole::TaskPrereq,
                    None,
                );
                self.named(
                    edges,
                    &value.services,
                    kind_names(Kinds::Service),
                    &value.span,
                    GraphRole::Runtime,
                    None,
                );
                self.named(
                    edges,
                    &value.checks,
                    kind_names(Kinds::Check),
                    &value.span,
                    GraphRole::Validation,
                    None,
                );
                if let Some(environment) = &value.environment {
                    self.edge(
                        edges,
                        environment,
                        kind_names(Kinds::Environment),
                        &value.span,
                        GraphRole::Runtime,
                        None,
                    );
                }
            }
            WorkspaceOutput::Schedule(value) => self.edge(
                edges,
                &value.task,
                kind_names(Kinds::Task),
                &value.span,
                GraphRole::Retention,
                None,
            ),
            WorkspaceOutput::Check(value) => {
                self.edge(
                    edges,
                    &value.subject,
                    kind_names(Kinds::All),
                    &value.span,
                    GraphRole::Retention,
                    None,
                );
                self.command(edges, &value.run, GraphRole::BuildInput, None);
            }
            WorkspaceOutput::Image(value) => {
                self.named(
                    edges,
                    &value.packages,
                    kind_names(Kinds::Package),
                    &value.span,
                    GraphRole::Runtime,
                    Some(&value.target),
                );
                self.arguments(
                    edges,
                    &value.config.entrypoint,
                    &value.span,
                    GraphRole::Runtime,
                    Some(&value.target),
                );
            }
            WorkspaceOutput::Profile(value) => {
                if let Some(environment) = &value.environment {
                    self.edge(
                        edges,
                        environment,
                        kind_names(Kinds::Environment),
                        &value.span,
                        GraphRole::Retention,
                        None,
                    );
                }
                self.named(
                    edges,
                    &value.schedules,
                    kind_names(Kinds::Schedule),
                    &value.span,
                    GraphRole::Retention,
                    None,
                );
                self.named(
                    edges,
                    &value.hooks,
                    kind_names(Kinds::Hook),
                    &value.span,
                    GraphRole::Retention,
                    None,
                );
                let mut i = 0;
                while i < value.files.len() {
                    self.file(edges, &value.files[i]);
                    i += 1;
                }
            }
            WorkspaceOutput::Hook(value) => {
                self.command(edges, &value.run, GraphRole::Runtime, None)
            }
            WorkspaceOutput::Service(value) => self.named(
                edges,
                &value.depends_on,
                kind_names(Kinds::Service),
                &value.span,
                GraphRole::Runtime,
                None,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn targets<'a>(edges: &[Reference<'a>], role: GraphRole) -> Vec<&'a str> {
        edges
            .iter()
            .filter(|edge| edge.role == role)
            .map(|edge| edge.to)
            .collect()
    }

    /// A profile file sourced from another output's tree depends on that output.
    #[test]
    fn a_tree_source_is_a_runtime_reference() {
        let profile: WorkspaceOutput = serde_json::from_value(json!({
            "kind":"profile","name":"home","span":{"file":"w.ts","line":1},
            "files":[{"span":{"file":"w.ts","line":2},
                "source":{"kind":"tree","output":"docs","include":["**"]},
                "content":{"kind":"identity"},
                "destination":{"kind":"tracked_copy","path":"~/docs"}}]
        }))
        .unwrap();
        let mut edges = Vec::new();
        Collector { output: &profile }.collect(&mut edges);
        assert_eq!(targets(&edges, GraphRole::Runtime), ["docs"]);
    }

    /// An image's entrypoint package command is a reference to that package.
    #[test]
    fn an_image_entrypoint_is_a_runtime_reference() {
        let image: WorkspaceOutput = serde_json::from_value(json!({
            "kind":"image","name":"image","span":{"file":"w.ts","line":1},
            "packages":["tool"],"target":{"os":"linux","arch":"x86_64"},
            "config":{"entrypoint":[{"kind":"package_command","package":"runner","command":"run"}]}
        }))
        .unwrap();
        let mut edges = Vec::new();
        Collector { output: &image }.collect(&mut edges);
        assert_eq!(targets(&edges, GraphRole::Runtime), ["tool", "runner"]);
        assert_eq!(edges[1].command, Some("run"));
    }
}
