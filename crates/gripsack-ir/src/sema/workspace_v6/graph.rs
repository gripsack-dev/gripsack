use super::{fail, values};
use crate::workspace_v6::graph::{bind_reference, references};
use crate::{Diagnostic, Span, codes, workspace_v6::*};
use gripsack_policy::{
    graph::{build_closure, roles::GraphRole},
    target::supports_target,
};
use std::collections::BTreeMap;

pub(super) fn check<'a>(
    workspace: &'a WorkspaceV6,
    catalog: &BTreeMap<&'a str, &'a WorkspaceOutput>,
    inputs: &BTreeMap<&str, &WorkspaceInput>,
    out: &mut Vec<Diagnostic>,
) {
    let names: Vec<&str> = catalog.keys().copied().collect();
    let indices: BTreeMap<&str, usize> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (*name, index))
        .collect();
    let edges = references(workspace);
    for output in &workspace.outputs {
        check_inputs_and_context(output, catalog, inputs, out);
        if let WorkspaceOutput::Image(image) = output {
            super::image::selection(image, catalog, out);
        }
    }
    let mut dependencies = vec![Vec::new(); names.len()];
    let mut production = vec![Vec::new(); names.len()];
    for edge in &edges {
        let Some(target) = catalog.get(edge.to) else {
            fail(
                out,
                codes::UNKNOWN_WORKSPACE_REF,
                edge.at,
                format!("unknown workspace output {:?}", edge.to),
            );
            continue;
        };
        if !edge.kinds.contains(&target.kind()) {
            fail(
                out,
                codes::UNKNOWN_WORKSPACE_REF,
                edge.at,
                format!(
                    "output {:?} is {}, expected {}",
                    edge.to,
                    target.kind(),
                    edge.kinds.join(" or ")
                ),
            );
            continue;
        }
        if let (Some(command), WorkspaceOutput::Package(package)) = (edge.command, *target)
            && !package.commands.contains_key(command)
        {
            fail(
                out,
                codes::UNKNOWN_WORKSPACE_REF,
                edge.at,
                format!("package {:?} does not export command {command:?}", edge.to),
            );
        }
        if edge
            .selector
            .is_some_and(|selector| !values::selector(selector))
        {
            fail(
                out,
                codes::INVALID_WORKSPACE_VALUE,
                edge.at,
                "invalid artifact selector",
            );
        }
        if let Some(available) = edge.target {
            let required = match target {
                WorkspaceOutput::Recipe(value) => Some(&value.target),
                WorkspaceOutput::Package(value) => Some(&value.target),
                _ => None,
            };
            if required.is_some_and(|required| {
                !supports_target(
                    &required.policy_requirement(),
                    &available.policy_requirement(),
                )
            }) {
                out.push(Diagnostic::error(codes::BAD_WORKSPACE_CONTEXT, "referenced artifact is incompatible with the consumer execution/target platform")
                    .with_label(Some(edge.at.clone()), "consumer declared here").with_label(Some(target.span().clone()), "artifact target declared here"));
            }
        }
        if let (WorkspaceOutput::Environment(environment), WorkspaceOutput::Package(package)) =
            (edge.from, *target)
        {
            match &package.layout {
                PackageLayoutV6::FixedPrefix { prefix } => {
                    if environment.prefix.as_ref() != Some(prefix) {
                        fail(
                            out,
                            codes::BAD_WORKSPACE_CONTEXT,
                            edge.at,
                            "fixed-prefix package cannot be selected at a different environment prefix",
                        );
                    }
                }
                PackageLayoutV6::PrefixMaterialized if environment.prefix.is_some() => {
                    fail(
                        out,
                        codes::BAD_WORKSPACE_CONTEXT,
                        edge.at,
                        "prefix-materialized packages derive their prefix from the frozen lock, not an environment prefix",
                    );
                }
                _ => {}
            }
        }
        let Some(bound) = bind_reference(
            &names,
            edge,
            indices.get(edge.from.name()).copied(),
            indices.get(edge.to).copied(),
        ) else {
            fail(
                out,
                codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                edge.at,
                "decoded reference lost its catalog identity",
            );
            continue;
        };
        if bound.dependency {
            dependencies[bound.from.position()].push(bound.to.position());
        }
        if bound.decision.build {
            production[bound.from.position()].push(bound.to.position());
        }
        if edge.role == GraphRole::Validation && !bound.decision.required_validation {
            fail(
                out,
                codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                edge.at,
                "a required publication/invocation check lost its validation role",
            );
        }
    }
    if out.is_empty() {
        if let Some(cycle) = cycle(&dependencies) {
            fail(
                out,
                codes::WORKSPACE_CYCLE,
                catalog[names[cycle]].span(),
                "workspace dependency cycle",
            );
        } else {
            let mut closures = BTreeMap::new();
            for edge in &edges {
                if matches!(edge.role, GraphRole::Production | GraphRole::BuildInput) {
                    let from = indices[edge.from.name()];
                    let to = indices[edge.to];
                    let closure = closures
                        .entry(from)
                        .or_insert_with(|| build_closure(names.len(), &production, from));
                    if !closure.contains(&to) {
                        fail(
                            out,
                            codes::REQUIRED_WORKSPACE_EDGE_MISSING,
                            edge.at,
                            "a declared producer/input is absent from the admitted production closure",
                        );
                    }
                }
            }
        }
    }
}

fn check_inputs_and_context(
    output: &WorkspaceOutput,
    catalog: &BTreeMap<&str, &WorkspaceOutput>,
    inputs: &BTreeMap<&str, &WorkspaceInput>,
    out: &mut Vec<Diagnostic>,
) {
    use crate::workspace_v6::graph::input::{
        argument_input_reference, output_source_input_references,
    };
    use gripsack_policy::workspace_command::{
        ArgumentOrigin, CommandOwner, DirectoryOrigin, ExecutionContext, admit_directory,
        admit_production_binding,
    };
    fn arg(
        value: &WorkspaceArg,
        at: &Span,
        inputs: &BTreeMap<&str, &WorkspaceInput>,
        out: &mut Vec<Diagnostic>,
    ) {
        if let Some(reference) = argument_input_reference(value, at) {
            let input = reference.to;
            if !inputs.contains_key(input) {
                fail(
                    out,
                    codes::UNKNOWN_WORKSPACE_REF,
                    reference.at,
                    format!("unknown captured input {input:?}"),
                );
            }
        }
    }
    let context = if matches!(
        output,
        WorkspaceOutput::Recipe(RecipeOutput {
            execution: RecipeExecution::IsolatedLinux { .. },
            ..
        })
    ) {
        ExecutionContext::IsolatedLinux
    } else {
        ExecutionContext::Host
    };
    let check_owner = |check: &crate::workspace_v6::CheckOutput| {
        if matches!(
            catalog.get(check.subject.as_str()),
            Some(WorkspaceOutput::Recipe(_) | WorkspaceOutput::Package(_))
        ) {
            CommandOwner::ImmutableSubject
        } else {
            CommandOwner::Invocation
        }
    };
    let owner = match output {
        WorkspaceOutput::Recipe(_) => CommandOwner::Production,
        WorkspaceOutput::Check(check) => check_owner(check),
        _ => CommandOwner::Invocation,
    };
    let command = |value: &WorkspaceCommand,
                   owner: CommandOwner,
                   check_target: Option<&WorkspacePlatform>,
                   out: &mut Vec<Diagnostic>| {
        for argument in value.arguments() {
            arg(argument, value.span(), inputs, out);
            if !admit_production_binding(owner, argument.policy_origin()) {
                fail(
                    out,
                    codes::BAD_WORKSPACE_CONTEXT,
                    value.span(),
                    "source bindings require production or an immutable recipe/package check subject; output bindings require a production owner",
                );
            }
            if let (Some(required), WorkspaceArg::PackageCommand { package, .. }) =
                (check_target, argument)
                && let Some(WorkspaceOutput::Package(provider)) = catalog.get(package.as_str())
                && !supports_target(
                    &provider.target.policy_requirement(),
                    &required.policy_requirement(),
                )
            {
                out.push(Diagnostic::error(codes::BAD_WORKSPACE_CONTEXT, "required check tool is incompatible with its production execution platform")
                    .with_label(Some(value.span().clone()), "check executes in this recipe's context")
                    .with_label(Some(provider.span.clone()), "tool target declared here"));
            }
        }
        let origin = match value.working_directory() {
            None | Some(WorkspacePath::Literal { .. }) => DirectoryOrigin::Literal,
            Some(WorkspacePath::Artifact { .. }) => DirectoryOrigin::Artifact,
            Some(WorkspacePath::Host { .. }) => DirectoryOrigin::LiveHost,
            Some(WorkspacePath::Source { .. }) => DirectoryOrigin::ProductionSource,
            Some(WorkspacePath::Output { .. }) => DirectoryOrigin::StagingOutput,
        };
        if !admit_directory(context, origin) {
            fail(
                out,
                codes::BAD_WORKSPACE_CONTEXT,
                value.span(),
                "isolated production and its required checks cannot acquire a live host working directory",
            );
        }
        let binding = match origin {
            DirectoryOrigin::ProductionSource => ArgumentOrigin::ProductionSource,
            DirectoryOrigin::StagingOutput => ArgumentOrigin::StagingOutput,
            _ => ArgumentOrigin::Literal,
        };
        if !admit_production_binding(owner, binding) {
            fail(
                out,
                codes::BAD_WORKSPACE_CONTEXT,
                value.span(),
                "production working-directory bindings are unavailable to native invocations",
            );
        }
    };
    for reference in output_source_input_references(output).into_iter().flatten() {
        if !inputs.contains_key(reference.to) {
            fail(
                out,
                codes::UNKNOWN_WORKSPACE_REF,
                reference.at,
                format!(
                    "pixi import names unknown captured input {:?}",
                    reference.to
                ),
            );
        }
    }
    match output {
        WorkspaceOutput::Recipe(value) => {
            for step in &value.steps {
                if let WorkspaceStep::Command(value) = step {
                    command(value, owner, None, out);
                }
            }
            let target = match &value.execution {
                RecipeExecution::Host { .. } => &value.target,
                RecipeExecution::IsolatedLinux { platform, .. } => platform,
            };
            for name in &value.checks {
                if let Some(WorkspaceOutput::Check(check)) = catalog.get(name.as_str()) {
                    command(&check.run, check_owner(check), Some(target), out);
                }
            }
        }
        WorkspaceOutput::Package(_) => {}
        WorkspaceOutput::Task(value) => {
            for step in &value.steps {
                if let WorkspaceStep::Command(value) = step {
                    command(value, owner, None, out);
                }
            }
            for check in &value.checks {
                if let Some(WorkspaceOutput::Check(check)) = catalog.get(check.as_str()) {
                    command(&check.run, check_owner(check), None, out);
                }
            }
        }
        WorkspaceOutput::Check(value) => command(&value.run, owner, None, out),
        WorkspaceOutput::Hook(value) => command(&value.run, owner, None, out),
        WorkspaceOutput::Environment(value) => {
            for value_arg in value.env.values() {
                arg(value_arg, &value.span, inputs, out);
                if !admit_production_binding(CommandOwner::Invocation, value_arg.policy_origin()) {
                    fail(
                        out,
                        codes::BAD_WORKSPACE_CONTEXT,
                        &value.span,
                        "a native environment has no production source/output binding",
                    );
                }
            }
        }
        _ => {}
    }
}
fn cycle(graph: &[Vec<usize>]) -> Option<usize> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Visit {
        Unseen,
        Active,
        Complete,
    }
    let mut states = vec![Visit::Unseen; graph.len()];
    for root in 0..graph.len() {
        if states[root] != Visit::Unseen {
            continue;
        }
        let mut stack = vec![(root, 0)];
        states[root] = Visit::Active;
        while let Some((node, edge)) = stack.last_mut() {
            if *edge == graph[*node].len() {
                states[*node] = Visit::Complete;
                stack.pop();
                continue;
            }
            let next = graph[*node][*edge];
            *edge += 1;
            match states[next] {
                Visit::Active => return Some(next),
                Visit::Unseen => {
                    states[next] = Visit::Active;
                    stack.push((next, 0));
                }
                Visit::Complete => {}
            }
        }
    }
    None
}
