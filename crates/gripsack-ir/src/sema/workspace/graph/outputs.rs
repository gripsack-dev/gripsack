//! Per-output edge roles and bindings; the catalog graph walker calls
//! this once per decoded output. Consumers and publication gates are
//! resolved but never silently converted into build prerequisites.

use super::{
    ARTIFACT_KINDS, CHECK, ENVIRONMENT, Edge, EdgeRole, HOOK, LocalOrder, PACKAGE, Projection,
    RECIPE, Relation, SCHEDULE, SUBJECT_KINDS, TASK, TargetBinding, command_edges, env_value,
    names_edges,
};
use crate::workspace::{WorkspaceOutput, WorkspaceProducer, WorkspaceSource};

pub(super) fn output_edges<'a>(output: &'a WorkspaceOutput, projection: &mut Projection<'a>) {
    match output {
        WorkspaceOutput::Recipe(recipe) => {
            for step in &recipe.steps {
                command_edges(step, output, EdgeRole::BuildInput, projection);
            }
            for (before, pair) in recipe.steps.windows(2).enumerate() {
                projection.ordering.push(LocalOrder {
                    recipe: &recipe.name,
                    before,
                    after: before + 1,
                    at: pair[1].span(),
                    role: EdgeRole::Ordering,
                });
            }
            names_edges(
                output,
                recipe.checks.iter(),
                CHECK,
                EdgeRole::Validation,
                Relation::field("publication check"),
                TargetBinding::None,
                projection,
            );
        }
        WorkspaceOutput::Package(package) => {
            match &package.producer {
                WorkspaceProducer::Recipe { recipe } => names_edges(
                    output,
                    std::iter::once(recipe),
                    RECIPE,
                    EdgeRole::Production,
                    Relation::field("producer"),
                    TargetBinding::Producer,
                    projection,
                ),
                // A provider-backed package acquires directly — no
                // catalog reference to check (0052 §2.2, A1-12).
                WorkspaceProducer::Provider { .. } => {}
            }
            names_edges(
                output,
                package.runtime.iter(),
                PACKAGE,
                EdgeRole::Runtime,
                Relation::field("runtime closure"),
                TargetBinding::None,
                projection,
            );
        }
        WorkspaceOutput::Environment(environment) => {
            names_edges(
                output,
                environment.packages.iter(),
                PACKAGE,
                EdgeRole::Runtime,
                Relation::field("package selection"),
                TargetBinding::Selection,
                projection,
            );
            for (variable, arg) in &environment.env {
                env_value(
                    arg,
                    output,
                    EdgeRole::Runtime,
                    Relation::variable("environment variable", variable),
                    &environment.span,
                    projection,
                );
            }
        }
        WorkspaceOutput::Task(task) => {
            command_edges(&task.run, output, EdgeRole::Runtime, projection);
            names_edges(
                output,
                task.deps.iter(),
                TASK,
                EdgeRole::TaskPrereq,
                Relation::field("prerequisite"),
                TargetBinding::None,
                projection,
            );
            names_edges(
                output,
                task.environment.iter(),
                ENVIRONMENT,
                EdgeRole::Runtime,
                Relation::field("environment"),
                TargetBinding::None,
                projection,
            );
            names_edges(
                output,
                task.checks.iter(),
                CHECK,
                EdgeRole::Validation,
                Relation::field("postcondition"),
                TargetBinding::None,
                projection,
            );
        }
        WorkspaceOutput::Schedule(schedule) => {
            names_edges(
                output,
                std::iter::once(&schedule.task),
                TASK,
                EdgeRole::Retention,
                Relation::field("task"),
                TargetBinding::None,
                projection,
            );
        }
        WorkspaceOutput::Check(check) => {
            command_edges(&check.run, output, EdgeRole::Runtime, projection);
            names_edges(
                output,
                std::iter::once(&check.subject),
                SUBJECT_KINDS,
                EdgeRole::Validation,
                Relation::field("subject"),
                TargetBinding::None,
                projection,
            );
        }
        WorkspaceOutput::Image(image) => {
            names_edges(
                output,
                image.packages.iter(),
                PACKAGE,
                EdgeRole::Runtime,
                Relation::field("package selection"),
                TargetBinding::Selection,
                projection,
            );
        }
        WorkspaceOutput::Profile(profile) => {
            for file in &profile.files {
                if let Some(WorkspaceSource::ArtifactFile {
                    output: to,
                    selector,
                }) = &file.source
                {
                    projection.edges.push(Edge {
                        from: output,
                        to,
                        role: EdgeRole::Runtime,
                        expected: ARTIFACT_KINDS,
                        relation: Relation::field("file source"),
                        at: &file.span,
                        selector: Some(selector),
                        command: None,
                        binding: TargetBinding::None,
                    });
                }
            }
            names_edges(
                output,
                profile.environment.iter(),
                ENVIRONMENT,
                EdgeRole::Retention,
                Relation::field("environment"),
                TargetBinding::None,
                projection,
            );
            names_edges(
                output,
                profile.schedules.iter(),
                SCHEDULE,
                EdgeRole::Retention,
                Relation::field("schedule"),
                TargetBinding::None,
                projection,
            );
            names_edges(
                output,
                profile.hooks.iter(),
                HOOK,
                EdgeRole::Retention,
                Relation::field("hook"),
                TargetBinding::None,
                projection,
            );
        }
        WorkspaceOutput::Hook(hook) => {
            command_edges(&hook.run, output, EdgeRole::Runtime, projection);
        }
    }
}
