//! One declared-reference projection shared by admission and realization.
//! The contracts are checked over the actual serde types; see `spec` for the
//! reference-bearing fields and explicit reference-free datatype boundaries.
use super::*;
use crate::Span;
use gripsack_policy::graph::roles::GraphRole;
#[allow(unused_imports)]
use gripsack_policy::graph::collector::*;
use std::collections::BTreeMap;
use vstd::prelude::*;
mod spec;
use spec::*;
mod kinds;
use kinds::*;
pub mod input;

verus! {
broadcast use vstd::seq_lib::group_seq_properties;
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
#[inline]
fn push<'a>(edges: &mut Vec<Reference<'a>>, edge: Reference<'a>)
    ensures views(final(edges)@) =~= views(old(edges)@).insert(edge_view(edge)),
{
    proof { expand_push(edges@, edge, |e: Reference<'a>| ISet::empty().insert(edge_view(e))); }
    edges.push(edge);
}

pub fn references(workspace: &WorkspaceV6) -> (edges: Vec<Reference<'_>>)
    requires vstd::std_specs::btree::key_obeys_cmp_spec::<String>(),
    ensures views(edges@) =~= workspace_edges(workspace.outputs@),
{
    let mut edges = Vec::new();
    let mut i = 0;
    while i < workspace.outputs.len()
        invariant i <= workspace.outputs.len(),
            views(edges@) =~= workspace_edges(workspace.outputs@.take(i as int)),
            vstd::std_specs::btree::key_obeys_cmp_spec::<String>(),
        decreases workspace.outputs.len() - i,
    {
        Collector { output: &workspace.outputs[i] }.collect(&mut edges);
        proof { expand_take(workspace.outputs@, i as int, |o: WorkspaceOutput| output_edges(&o)); }
        i += 1;
    }
    proof { assert(workspace.outputs@.take(workspace.outputs@.len() as int) =~= workspace.outputs@); }
    edges
}

struct Collector<'a> { output: &'a WorkspaceOutput }
impl<'a> Collector<'a> {
    fn edge(&self, edges: &mut Vec<Reference<'a>>, to: &'a str, kinds: &'static [&'static str],
        at: &'a Span, role: GraphRole, target: Option<&'a WorkspacePlatform>)
        ensures views(final(edges)@) =~= views(old(edges)@).insert(bare(self.output, to@,
            kinds@.map_values(|s: &str| s@), at, role, target)),
    {
        push(edges, Reference { from: self.output, to, kinds, at, role,
            command: None, selector: None, target });
    }
    fn named(&self, edges: &mut Vec<Reference<'a>>, names: &'a [String], kinds: &'static [&'static str],
        at: &'a Span, role: GraphRole, target: Option<&'a WorkspacePlatform>)
        ensures views(final(edges)@) =~= views(old(edges)@).union(named_edges(self.output, names@,
            kinds@.map_values(|s: &str| s@), at, role, target)),
    {
        let ghost initial = views(edges@);
        let mut i = 0;
        while i < names.len()
            invariant i <= names.len(), views(edges@) =~= initial.union(named_edges(self.output,
                names@.take(i as int), kinds@.map_values(|s: &str| s@), at, role, target)),
            decreases names.len() - i,
        {
            self.edge(edges, &names[i], kinds, at, role, target);
            proof { expand_take(names@, i as int, |name: String| ISet::empty().insert(
                bare(self.output, name@, kinds@.map_values(|s: &str| s@), at, role, target))); }
            i += 1;
        }
        proof { assert(names@.take(names@.len() as int) =~= names@); }
    }
    fn argument(&self, edges: &mut Vec<Reference<'a>>, value: &'a WorkspaceArg, at: &'a Span,
        role: GraphRole, target: Option<&'a WorkspacePlatform>)
        ensures views(final(edges)@) =~= views(old(edges)@).union(argument_edges(self.output, *value, at, role, target)),
    {
        match value {
            WorkspaceArg::Artifact { output, selector } => {
                let edge = Reference { from: self.output, to: output, kinds: &["recipe", "package"], at, role,
                    command: None, selector: Some(selector), target: None };
                proof { assert(edge.kinds@.map_values(|s: &str| s@) =~= artifact_kinds()); }
                push(edges, edge);
            }
            WorkspaceArg::PackageCommand { package, command, .. } => {
                let edge = Reference { from: self.output, to: package, kinds: &["package"], at, role,
                    command: Some(command), selector: None, target };
                proof { assert(edge.kinds@.map_values(|s: &str| s@) =~= package_kinds()); }
                push(edges, edge);
            }
            WorkspaceArg::Literal { .. } | WorkspaceArg::Input { .. }
            | WorkspaceArg::Source { .. } | WorkspaceArg::Output { .. } => {}
        }
    }
    fn arguments(&self, edges: &mut Vec<Reference<'a>>, args: &'a [WorkspaceArg], at: &'a Span,
        role: GraphRole, target: Option<&'a WorkspacePlatform>)
        ensures views(final(edges)@) =~= views(old(edges)@).union(arguments_edges(self.output, args@.to_set(), at, role, target)),
    {
        let ghost initial = views(edges@);
        let mut i = 0;
        while i < args.len()
            invariant i <= args.len(), views(edges@) =~= initial.union(arguments_edges(self.output,
                args@.take(i as int).to_set(), at, role, target)),
            decreases args.len() - i,
        {
            self.argument(edges, &args[i], at, role, target);
            proof { expand_take(args@, i as int, |arg: WorkspaceArg| argument_edges(self.output, arg, at, role, target)); }
            i += 1;
        }
        proof { assert(args@.take(args@.len() as int) =~= args@); }
    }
    fn environment(&self, edges: &mut Vec<Reference<'a>>, env: &'a BTreeMap<String, WorkspaceArg>,
        at: &'a Span, role: GraphRole, target: Option<&'a WorkspacePlatform>)
        requires vstd::std_specs::btree::key_obeys_cmp_spec::<String>(),
        ensures views(final(edges)@) =~= views(old(edges)@).union(arguments_edges(self.output, env@.values(), at, role, target)),
    {
        let ghost initial = views(edges@);
        for argument in iter: env.values()
            invariant views(edges@) =~= initial.union(arguments_edges(self.output,
                iter.history().unref().to_set(), at, role, target)),
                iter.seq().unref().to_set() == env@.values(),
                iter.history() =~= iter.seq().take(iter.index()),
                iter.seq().take(iter.seq().len() as int) =~= iter.seq(),
        {
            self.argument(edges, argument, at, role, target);
            proof {
                assert(iter.history().push(argument).unref() =~= iter.history().unref().push(*argument));
                expand_push(iter.history().unref(), *argument, |arg: WorkspaceArg| argument_edges(self.output, arg, at, role, target));
            }
        }
    }
    fn command(&self, edges: &mut Vec<Reference<'a>>, value: &'a WorkspaceCommand,
        role: GraphRole, target: Option<&'a WorkspacePlatform>)
        requires vstd::std_specs::btree::key_obeys_cmp_spec::<String>(),
        ensures views(final(edges)@) =~= views(old(edges)@).union(command_edges(self.output, value, role, target)),
    {
        let (env, cwd, at) = match value {
            WorkspaceCommand::Exec { argv, env, cwd, span } => {
                self.arguments(edges, argv, span, role, target);
                proof { expand_union(argv@.to_set(), env@.values(), |arg: WorkspaceArg| argument_edges(self.output, arg, span, role, target)); }
                (env, cwd, span)
            }
            WorkspaceCommand::RunBash { interpreter, env, cwd, span, .. } => {
                self.argument(edges, interpreter, span, role, target);
                proof { expand_insert(env@.values(), *interpreter, |arg: WorkspaceArg| argument_edges(self.output, arg, span, role, target)); }
                (env, cwd, span)
            }
        };
        self.environment(edges, env, at, role, target);
        if let Some(WorkspacePath::Artifact { output, selector }) = cwd {
            push(edges, Reference { from: self.output, to: output, kinds: kind_names(Kinds::Artifact),
                at, role, command: None, selector: Some(selector), target: None });
        }
    }
    fn steps(&self, edges: &mut Vec<Reference<'a>>, values: &'a [WorkspaceStep],
        role: GraphRole, target: Option<&'a WorkspacePlatform>)
        requires vstd::std_specs::btree::key_obeys_cmp_spec::<String>(),
        ensures views(final(edges)@) =~= views(old(edges)@).union(steps_edges(self.output, values@, role, target)),
    {
        let ghost initial = views(edges@);
        let mut i = 0;
        while i < values.len()
            invariant i <= values.len(), views(edges@) =~= initial.union(steps_edges(self.output,
                values@.take(i as int), role, target)),
                vstd::std_specs::btree::key_obeys_cmp_spec::<String>(),
            decreases values.len() - i,
        {
            match &values[i] {
                WorkspaceStep::Command(command) => self.command(edges, command, role, target),
                WorkspaceStep::Action(WorkspaceAction::EnsureArtifact { output, span }) =>
                    self.edge(edges, output, kind_names(Kinds::Artifact), span, role, None),
            }
            proof { expand_take(values@, i as int, |step: WorkspaceStep| step_edges(self.output, &step, role, target)); }
            i += 1;
        }
        proof { assert(values@.take(values@.len() as int) =~= values@); }
    }
    fn file(&self, edges: &mut Vec<Reference<'a>>, file: &'a WorkspaceFile)
        ensures views(final(edges)@) =~= views(old(edges)@).union(file_edges(self.output, file)),
    {
        match &file.source {
            Some(WorkspaceSource::ArtifactFile { output, selector }) => push(edges, Reference {
                from: self.output, to: output, kinds: kind_names(Kinds::Artifact), at: &file.span,
                role: GraphRole::Runtime, command: None, selector: Some(selector), target: None }),
            Some(WorkspaceSource::Tree { output, .. }) => self.edge(edges, output,
                kind_names(Kinds::Artifact), &file.span, GraphRole::Runtime, None),
            Some(WorkspaceSource::RepoFile { .. }) | None => {}
        }
        let ghost initial = views(edges@);
        let mut i = 0;
        while i < file.checks.len()
            invariant i <= file.checks.len(), views(edges@) =~= initial.union(
                file_check_edges(self.output, file.checks@.take(i as int))),
            decreases file.checks.len() - i,
        {
            let check = &file.checks[i];
            self.edge(edges, &check.check, kind_names(Kinds::Check), &check.span, GraphRole::Validation, None);
            proof { expand_take(file.checks@, i as int, |check: WorkspaceFileCheck| ISet::empty().insert(
                bare(self.output, check.check@, seq!["check"@], &check.span, GraphRole::Validation, None))); }
            i += 1;
        }
        proof { assert(file.checks@.take(file.checks@.len() as int) =~= file.checks@); }
    }
    fn collect(&self, edges: &mut Vec<Reference<'a>>)
        requires vstd::std_specs::btree::key_obeys_cmp_spec::<String>(),
        ensures views(final(edges)@) =~= views(old(edges)@).union(output_edges(self.output)),
    {
        match self.output {
            WorkspaceOutput::Recipe(value) => {
                let execution = match &value.execution {
                    RecipeExecution::IsolatedLinux { platform, .. } => platform,
                    RecipeExecution::Host { .. } => &value.target,
                };
                self.steps(edges, &value.steps, GraphRole::BuildInput, Some(execution));
                self.named(edges, &value.checks, kind_names(Kinds::Check), &value.span, GraphRole::Validation, None);
            }
            WorkspaceOutput::Package(value) => {
                if let WorkspaceProducer::Recipe { recipe } = &value.producer {
                    self.edge(edges, recipe, kind_names(Kinds::Recipe), &value.span, GraphRole::Production, Some(&value.target));
                }
                self.named(edges, &value.runtime, kind_names(Kinds::Package), &value.span, GraphRole::Runtime, Some(&value.target));
            }
            WorkspaceOutput::Environment(value) => {
                self.named(edges, &value.packages, kind_names(Kinds::Package), &value.span, GraphRole::Runtime, Some(&value.target));
                self.environment(edges, &value.env, &value.span, GraphRole::Runtime, Some(&value.target));
            }
            WorkspaceOutput::Task(value) => {
                self.steps(edges, &value.steps, GraphRole::Runtime, None);
                self.named(edges, &value.deps, kind_names(Kinds::Task), &value.span, GraphRole::TaskPrereq, None);
                self.named(edges, &value.checks, kind_names(Kinds::Check), &value.span, GraphRole::Validation, None);
                if let Some(environment) = &value.environment {
                    self.edge(edges, environment, kind_names(Kinds::Environment), &value.span, GraphRole::Runtime, None);
                }
            }
            WorkspaceOutput::Schedule(value) => self.edge(edges, &value.task, kind_names(Kinds::Task), &value.span, GraphRole::Retention, None),
            WorkspaceOutput::Check(value) => {
                self.edge(edges, &value.subject, kind_names(Kinds::All),
                    &value.span, GraphRole::Retention, None);
                self.command(edges, &value.run, GraphRole::BuildInput, None);
            }
            WorkspaceOutput::Image(value) => {
                self.named(edges, &value.packages, kind_names(Kinds::Package), &value.span, GraphRole::Runtime, Some(&value.target));
                self.arguments(edges, &value.config.entrypoint, &value.span, GraphRole::Runtime, Some(&value.target));
            }
            WorkspaceOutput::Profile(value) => {
                if let Some(environment) = &value.environment {
                    self.edge(edges, environment, kind_names(Kinds::Environment), &value.span, GraphRole::Retention, None);
                }
                self.named(edges, &value.schedules, kind_names(Kinds::Schedule), &value.span, GraphRole::Retention, None);
                self.named(edges, &value.hooks, kind_names(Kinds::Hook), &value.span, GraphRole::Retention, None);
                let ghost initial = views(edges@);
                let mut i = 0;
                while i < value.files.len()
                    invariant i <= value.files.len(), views(edges@) =~= initial.union(
                        files_edges(self.output, value.files@.take(i as int))),
                    decreases value.files.len() - i,
                {
                    self.file(edges, &value.files[i]);
                    proof { expand_take(value.files@, i as int, |file: WorkspaceFile| file_edges(self.output, &file)); }
                    i += 1;
                }
                proof { assert(value.files@.take(value.files@.len() as int) =~= value.files@); }
            }
            WorkspaceOutput::Hook(value) => self.command(edges, &value.run, GraphRole::Runtime, None),
        }
    }
}
}
