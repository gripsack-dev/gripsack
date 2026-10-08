//! Specifications of the actual serde datatypes, not a second input grammar.
//! Transparent wrappers make the existing fields visible to Verus. Opaque
//! leaves below contain no catalog/input references and are carried unchanged.
use super::*;

verus! {
#[verifier::external_type_specification] pub struct ExWorkspace(WorkspaceCatalog);
#[verifier::external_type_specification] pub struct ExOutput(WorkspaceOutput);
#[verifier::external_type_specification] pub struct ExRecipe(RecipeOutput);
#[verifier::external_type_specification] pub struct ExExecution(RecipeExecution);
#[verifier::external_type_specification] pub struct ExPackage(PackageOutput);
#[verifier::external_type_specification] pub struct ExProducer(WorkspaceProducer);
#[verifier::external_type_specification] pub struct ExEnvironment(EnvironmentOutput);
#[verifier::external_type_specification] pub struct ExTask(TaskOutput);
#[verifier::external_type_specification] pub struct ExSchedule(ScheduleOutput);
#[verifier::external_type_specification] pub struct ExCheck(CheckOutput);
#[verifier::external_type_specification] pub struct ExImage(ImageOutput);
#[verifier::external_type_specification] pub struct ExImageConfig(ImageRuntimeConfig);
#[verifier::external_type_specification] pub struct ExProfile(ProfileOutput);
#[verifier::external_type_specification] pub struct ExHook(HookOutput);
#[verifier::external_type_specification] pub struct ExArg(WorkspaceArg);
#[verifier::external_type_specification] pub struct ExCommand(WorkspaceCommand);
#[verifier::external_type_specification] pub struct ExPath(WorkspacePath);
#[verifier::external_type_specification] pub struct ExStep(WorkspaceStep);
#[verifier::external_type_specification] pub struct ExAction(WorkspaceAction);
#[verifier::external_type_specification] pub struct ExSource(AcquisitionSource);
#[verifier::external_type_specification] pub struct ExConda(CondaEnvironmentSource);
#[verifier::external_type_specification] pub struct ExPixi(PixiLockSource);
#[verifier::external_type_specification] pub struct ExFileSource(WorkspaceSource);
#[verifier::external_type_specification] pub struct ExFile(WorkspaceFile);
#[verifier::external_type_specification] pub struct ExFileCheck(WorkspaceFileCheck);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExSpan(Span);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExPlatform(WorkspacePlatform);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExFetch(crate::workspace::WorkspaceFetch);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExAccess(HostAccess);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExWorker(LinuxWorker);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExToolchain(ToolchainReference);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExOutputKind(RecipeOutputKind);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExLayout(CatalogPackageLayout);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExContext(TaskContext);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExCalendar(WorkspaceCalendar);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExScope(ScheduleScope);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExHookTrigger(HookTrigger);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExContent(WorkspaceContent);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExDestination(WorkspaceDestination);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExSubject(FileCheckSubject);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExStage(FileCheckStage);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExPrefix(InstallPrefix);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExImageDestination(ImageDestination);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExImageOwner(ImageOwner);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExInput(WorkspaceInput);
#[verifier::external_type_specification] #[verifier::external_body] pub struct ExLock(WorkspaceMutationLock);

pub struct EdgeView<'a> {
    pub from: &'a WorkspaceOutput,
    pub to: Seq<char>,
    pub kinds: Seq<Seq<char>>,
    pub at: &'a Span,
    pub role: GraphRole,
    pub command: Option<Seq<char>>,
    pub selector: Option<Seq<char>>,
    pub target: Option<&'a WorkspacePlatform>,
}

pub open spec fn edge_view<'a>(edge: Reference<'a>) -> EdgeView<'a> {
    EdgeView { from: edge.from, to: edge.to@,
        kinds: edge.kinds@.map_values(|s: &str| s@), at: edge.at, role: edge.role,
        command: match edge.command { Some(s) => Some(s@), None => None },
        selector: match edge.selector { Some(s) => Some(s@), None => None }, target: edge.target }
}
pub open spec fn views<'a>(edges: Seq<Reference<'a>>) -> ISet<EdgeView<'a>> {
    expand_seq(edges, |edge: Reference<'a>| ISet::empty().insert(edge_view(edge)))
}
pub open spec fn artifact_kinds() -> Seq<Seq<char>> { seq!["recipe"@, "package"@] }
pub open spec fn package_kinds() -> Seq<Seq<char>> { seq!["package"@] }
pub open spec fn all_kinds() -> Seq<Seq<char>> {
    seq!["recipe"@, "package"@, "environment"@, "task"@, "schedule"@,
        "check"@, "image"@, "profile"@, "hook"@]
}
pub open spec fn bare<'a>(owner: &'a WorkspaceOutput, name: Seq<char>, kinds: Seq<Seq<char>>,
    at: &'a Span, role: GraphRole, target: Option<&'a WorkspacePlatform>) -> EdgeView<'a> {
    EdgeView { from: owner, to: name, kinds, at, role, command: None, selector: None, target }
}
pub open spec fn named_edges<'a>(owner: &'a WorkspaceOutput, names: Seq<String>, kinds: Seq<Seq<char>>,
    at: &'a Span, role: GraphRole, target: Option<&'a WorkspacePlatform>) -> ISet<EdgeView<'a>> {
    expand_seq(names, |name: String| ISet::empty().insert(bare(owner, name@, kinds, at, role, target)))
}
pub open spec fn argument_edges<'a>(owner: &'a WorkspaceOutput, value: WorkspaceArg, at: &'a Span,
    role: GraphRole, target: Option<&'a WorkspacePlatform>) -> ISet<EdgeView<'a>> {
    match value {
        WorkspaceArg::Artifact { output, selector } => ISet::empty().insert(EdgeView {
            selector: Some(selector@), ..bare(owner, output@, artifact_kinds(), at, role, None) }),
        WorkspaceArg::PackageCommand { package, command, .. } => ISet::empty().insert(EdgeView {
            command: Some(command@), ..bare(owner, package@, package_kinds(), at, role, target) }),
        WorkspaceArg::Literal { .. } | WorkspaceArg::Input { .. } | WorkspaceArg::Source { .. } | WorkspaceArg::Output { .. } => ISet::empty(),
    }
}
pub open spec fn arguments_edges<'a>(owner: &'a WorkspaceOutput, args: Set<WorkspaceArg>, at: &'a Span,
    role: GraphRole, target: Option<&'a WorkspacePlatform>) -> ISet<EdgeView<'a>> {
    expand(args, |arg: WorkspaceArg| argument_edges(owner, arg, at, role, target))
}
pub open spec fn path_edges<'a>(owner: &'a WorkspaceOutput, cwd: Option<WorkspacePath>, at: &'a Span,
    role: GraphRole) -> ISet<EdgeView<'a>> {
    match cwd {
        Some(WorkspacePath::Artifact { output, selector }) => ISet::empty().insert(EdgeView {
            selector: Some(selector@), ..bare(owner, output@, artifact_kinds(), at, role, None) }),
        _ => ISet::empty(),
    }
}
pub open spec fn command_edges<'a>(owner: &'a WorkspaceOutput, command: &'a WorkspaceCommand,
    role: GraphRole, target: Option<&'a WorkspacePlatform>) -> ISet<EdgeView<'a>> {
    match command {
        WorkspaceCommand::Exec { span, argv, env, cwd } =>
            arguments_edges(owner, argv@.to_set().union(env@.values()), span, role, target)
                .union(path_edges(owner, *cwd, span, role)),
        WorkspaceCommand::RunBash { span, interpreter, env, cwd, .. } =>
            arguments_edges(owner, env@.values().insert(*interpreter), span, role, target)
                .union(path_edges(owner, *cwd, span, role)),
    }
}
pub open spec fn step_edges<'a>(owner: &'a WorkspaceOutput, step: &'a WorkspaceStep,
    role: GraphRole, target: Option<&'a WorkspacePlatform>) -> ISet<EdgeView<'a>> {
    match step {
        WorkspaceStep::Command(command) => command_edges(owner, command, role, target),
        WorkspaceStep::Action(WorkspaceAction::EnsureArtifact { output, span }) =>
            ISet::empty().insert(bare(owner, output@, artifact_kinds(), span, role, None)),
    }
}
pub open spec fn steps_edges<'a>(owner: &'a WorkspaceOutput, steps: Seq<WorkspaceStep>,
    role: GraphRole, target: Option<&'a WorkspacePlatform>) -> ISet<EdgeView<'a>> {
    expand_seq(steps, |step: WorkspaceStep| step_edges(owner, &step, role, target))
}
pub open spec fn file_edges<'a>(owner: &'a WorkspaceOutput, file: &'a WorkspaceFile) -> ISet<EdgeView<'a>> {
    let origin = match &file.source {
        Some(WorkspaceSource::ArtifactFile { output, selector }) => ISet::empty().insert(EdgeView {
            selector: Some(selector@), ..bare(owner, output@, artifact_kinds(), &file.span, GraphRole::Runtime, None) }),
        Some(WorkspaceSource::Tree { output, .. }) => ISet::empty().insert(
            bare(owner, output@, artifact_kinds(), &file.span, GraphRole::Runtime, None)),
        Some(WorkspaceSource::RepoFile { .. }) | None => ISet::empty(),
    };
    origin.union(file_check_edges(owner, file.checks@))
}
pub open spec fn file_check_edges<'a>(owner: &'a WorkspaceOutput, checks: Seq<WorkspaceFileCheck>) -> ISet<EdgeView<'a>> {
    expand_seq(checks, |check: WorkspaceFileCheck| ISet::empty().insert(
        bare(owner, check.check@, seq!["check"@], &check.span, GraphRole::Validation, None)))
}
pub open spec fn files_edges<'a>(owner: &'a WorkspaceOutput, files: Seq<WorkspaceFile>) -> ISet<EdgeView<'a>> {
    expand_seq(files, |file: WorkspaceFile| file_edges(owner, &file))
}
pub open spec fn output_edges<'a>(owner: &'a WorkspaceOutput) -> ISet<EdgeView<'a>> {
    match owner {
        WorkspaceOutput::Recipe(v) => {
            let target = match &v.execution {
                RecipeExecution::Host { .. } => &v.target,
                RecipeExecution::IsolatedLinux { platform, .. } => platform,
            };
            steps_edges(owner, v.steps@, GraphRole::BuildInput, Some(target))
                .union(named_edges(owner, v.checks@, seq!["check"@], &v.span, GraphRole::Validation, None))
        },
        WorkspaceOutput::Package(v) => {
            let producer = match &v.producer {
                WorkspaceProducer::Recipe { recipe } => ISet::empty().insert(bare(owner, recipe@, seq!["recipe"@], &v.span, GraphRole::Production, Some(&v.target))),
                WorkspaceProducer::Provider { .. } => ISet::empty(),
            };
            producer.union(named_edges(owner, v.runtime@, package_kinds(), &v.span, GraphRole::Runtime, Some(&v.target)))
        },
        WorkspaceOutput::Environment(v) =>
            named_edges(owner, v.packages@, package_kinds(), &v.span, GraphRole::Runtime, Some(&v.target))
                .union(arguments_edges(owner, v.env@.values(), &v.span, GraphRole::Runtime, Some(&v.target))),
        WorkspaceOutput::Task(v) =>
            steps_edges(owner, v.steps@, GraphRole::Runtime, None)
                .union(named_edges(owner, v.deps@, seq!["task"@], &v.span, GraphRole::TaskPrereq, None))
                .union(named_edges(owner, v.checks@, seq!["check"@], &v.span, GraphRole::Validation, None))
                .union(match &v.environment {
                    Some(name) => ISet::empty().insert(bare(owner, name@, seq!["environment"@], &v.span, GraphRole::Runtime, None)),
                    None => ISet::empty(),
                }),
        WorkspaceOutput::Schedule(v) => ISet::empty().insert(bare(owner, v.task@, seq!["task"@], &v.span, GraphRole::Retention, None)),
        WorkspaceOutput::Check(v) => ISet::empty().insert(bare(owner, v.subject@, all_kinds(), &v.span, GraphRole::Retention, None))
            .union(command_edges(owner, &v.run, GraphRole::BuildInput, None)),
        WorkspaceOutput::Image(v) => named_edges(owner, v.packages@, package_kinds(), &v.span, GraphRole::Runtime, Some(&v.target))
            .union(arguments_edges(owner, v.config.entrypoint@.to_set(), &v.span, GraphRole::Runtime, Some(&v.target))),
        WorkspaceOutput::Profile(v) => {
            let environment = match &v.environment {
                Some(name) => ISet::empty().insert(bare(owner, name@, seq!["environment"@], &v.span, GraphRole::Retention, None)),
                None => ISet::empty(),
            };
            environment
                .union(named_edges(owner, v.schedules@, seq!["schedule"@], &v.span, GraphRole::Retention, None))
                .union(named_edges(owner, v.hooks@, seq!["hook"@], &v.span, GraphRole::Retention, None))
                .union(files_edges(owner, v.files@))
        },
        WorkspaceOutput::Hook(v) => command_edges(owner, &v.run, GraphRole::Runtime, None),
    }
}
pub open spec fn workspace_edges<'a>(outputs: Seq<WorkspaceOutput>) -> ISet<EdgeView<'a>> {
    expand_seq(outputs, |output: WorkspaceOutput| output_edges(&output))
}
}
