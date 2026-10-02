//! The named-output catalog (0052 §2.1): the workspace envelope entry,
//! the nine output types, plus producer, execution and lifecycle
//! variants. Layout and target types live in cohesive sibling modules.
//! `outputs` names form the single declaration namespace — a collision
//! reports both declaration spans (sema E125).

use super::command::{WorkspaceArg, WorkspaceCommand};
use super::file::{WorkspaceCalendar, WorkspaceFile};
use super::layout::{InstallPrefix, PackageLayout};
use super::platform::WorkspacePlatform;
use crate::model::FetchSpec;
use crate::span::Span;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A recipe's payload source: the shared v3 fetch grammar plus the
/// mandatory provenance carried by v4/v5 workspace sources (schema
/// `$defs/workspaceFetch`). Historical module maps keep the bare v3
/// fetch shape — this wrapper is workspace-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFetch {
    pub fetch: FetchSpec,
    pub span: Span,
}

/// What produces a package's payload (plan/0052 §2.2: resolution and
/// acquisition stay separated — schema `$defs/workspaceProducer`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceProducer {
    /// Name of the `recipe` output that builds this package (sema E126).
    Recipe { recipe: String },
    /// A direct provider acquisition — the shared fetch grammar with
    /// mandatory provenance. A provider-backed package needs no
    /// synthetic recipe output.
    Provider { provider: WorkspaceFetch },
}

/// Current v5 workspace entry: one catalog of named outputs.
/// `outputs` names are the single declaration namespace — a collision is
/// an admission error naming both declaration spans (sema E125).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    /// Where the workspace itself was declared. Mandatory in v5.
    pub span: Span,
    /// The named-output catalog (schema: minItems 1, sema E130).
    pub outputs: Vec<WorkspaceOutput>,
}

/// One declared workspace output. The `kind` tag is closed by the
/// tagged-field pass; the nine variants are the current v5 grammar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceOutput {
    Recipe(RecipeOutput),
    Package(PackageOutput),
    Environment(EnvironmentOutput),
    Task(TaskOutput),
    Schedule(ScheduleOutput),
    Check(CheckOutput),
    Image(ImageOutput),
    Profile(ProfileOutput),
    Hook(HookOutput),
}

impl WorkspaceOutput {
    /// The catalog name — the single declaration namespace.
    pub fn name(&self) -> &str {
        match self {
            WorkspaceOutput::Recipe(o) => &o.name,
            WorkspaceOutput::Package(o) => &o.name,
            WorkspaceOutput::Environment(o) => &o.name,
            WorkspaceOutput::Task(o) => &o.name,
            WorkspaceOutput::Schedule(o) => &o.name,
            WorkspaceOutput::Check(o) => &o.name,
            WorkspaceOutput::Image(o) => &o.name,
            WorkspaceOutput::Profile(o) => &o.name,
            WorkspaceOutput::Hook(o) => &o.name,
        }
    }

    /// The wire `kind` spelling (`"recipe"`, `"package"`, …).
    pub fn kind(&self) -> &'static str {
        match self {
            WorkspaceOutput::Recipe(_) => "recipe",
            WorkspaceOutput::Package(_) => "package",
            WorkspaceOutput::Environment(_) => "environment",
            WorkspaceOutput::Task(_) => "task",
            WorkspaceOutput::Schedule(_) => "schedule",
            WorkspaceOutput::Check(_) => "check",
            WorkspaceOutput::Image(_) => "image",
            WorkspaceOutput::Profile(_) => "profile",
            WorkspaceOutput::Hook(_) => "hook",
        }
    }

    /// The mandatory declaration span.
    pub fn span(&self) -> &Span {
        match self {
            WorkspaceOutput::Recipe(o) => &o.span,
            WorkspaceOutput::Package(o) => &o.span,
            WorkspaceOutput::Environment(o) => &o.span,
            WorkspaceOutput::Task(o) => &o.span,
            WorkspaceOutput::Schedule(o) => &o.span,
            WorkspaceOutput::Check(o) => &o.span,
            WorkspaceOutput::Image(o) => &o.span,
            WorkspaceOutput::Profile(o) => &o.span,
            WorkspaceOutput::Hook(o) => &o.span,
        }
    }
}

/// A build description: source bytes plus an ordered local command list,
/// producing a file or tree artifact for one target platform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeOutput {
    pub name: String,
    pub span: Span,
    /// How to obtain the payload — the shared fetch grammar with its
    /// own declaration span.
    pub source: WorkspaceFetch,
    pub execution: RecipeExecution,
    pub output_kind: RecipeOutputKind,
    pub target: WorkspacePlatform,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<WorkspaceCommand>,
    /// Publication gates — names of `check` outputs (sema E126).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<String>,
}

/// A recipe declares its execution boundary; neither variant executes
/// in A1. Native downloads belong to provider-backed packages instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecipeExecution {
    Host { access: HostAccess },
    IsolatedLinux { worker: LinuxWorker },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostAccess {
    /// No filesystem, kernel or network isolation claim.
    Unconfined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinuxWorker {
    /// Provisioned privately by the later B2 executor, not by preview.
    Buildkit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeOutputKind {
    File,
    Tree,
}

/// A consumable artifact: a producer reference plus the commands it
/// exports. Command references imply the artifact and its runtime
/// closure (0052 §2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageOutput {
    pub name: String,
    pub span: Span,
    /// The producer: a recipe reference, or a direct provider
    /// acquisition (sema E126 checks the recipe variant's reference).
    pub producer: WorkspaceProducer,
    /// Exported command name → wire spelling (opaque to the core).
    pub commands: BTreeMap<String, String>,
    /// Explicit runtime closure — names of `package` outputs (sema E126).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime: Vec<String>,
    pub target: WorkspacePlatform,
    pub layout: PackageLayout,
}

/// An ordered package selection for one target — process-scoped, never
/// a personal-profile deployment (0052 §2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentOutput {
    pub name: String,
    pub span: Span,
    /// Names of `package` outputs (sema E126).
    pub packages: Vec<String>,
    pub target: WorkspacePlatform,
    /// Required destination for prefix-bound packages selected here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<InstallPrefix>,
    /// Environment contributions; values are data (literal or artifact
    /// reference), never command invocations (sema E128).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, WorkspaceArg>,
}

/// One invocable unit of work. `deps` are unordered prerequisites
/// verified within one invocation (E1 owns runtime); cycles are an
/// admission error (sema E127).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskOutput {
    pub name: String,
    pub span: Span,
    pub run: WorkspaceCommand,
    /// Names of `task` outputs (sema E126).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<String>,
    /// Name of an `environment` output (sema E126).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// Per-invocation postconditions — names of `check` outputs,
    /// distinct from build/publication checks (sema E126).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<String>,
}

/// An inert schedule declaration. Registration is deferred to E2/E3 and
/// rejected before then by the CLI lane (E124); the declaration alone
/// activates nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleOutput {
    pub name: String,
    pub span: Span,
    /// Name of a `task` output (sema E126).
    pub task: String,
    pub trigger: WorkspaceCalendar,
    pub scope: ScheduleScope,
}

/// The only scope these workspace readers admit; system/root scope
/// is rejected by grammar, never admitted as an inert promise (§2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleScope {
    User,
}

/// A check over a typed subject. A check command may have effects and is
/// never run by a read-only preview (0052 §2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckOutput {
    pub name: String,
    pub span: Span,
    pub run: WorkspaceCommand,
    /// The checked subject (opaque to structural admission).
    pub subject: String,
}

/// A machine image selection: packages plus target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageOutput {
    pub name: String,
    pub span: Span,
    /// Names of `package` outputs (sema E126).
    pub packages: Vec<String>,
    pub target: WorkspacePlatform,
}

/// The personal-profile output: owned files, an optional environment
/// selection, schedule and hook wiring. Generations, ownership, journal
/// and recovery stay authoritative (0052 §2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileOutput {
    pub name: String,
    pub span: Span,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<WorkspaceFile>,
    /// Name of an `environment` output (sema E126).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// Names of `schedule` outputs (sema E126).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedules: Vec<String>,
    /// Names of `hook` outputs (sema E126).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<String>,
}

/// A lifecycle hook. `post_link`/`post_activate`/`on_remove` semantics
/// are preserved from the live v3 `Trigger` enum (0052 §2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookOutput {
    pub name: String,
    pub span: Span,
    pub run: WorkspaceCommand,
    pub trigger: HookTrigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookTrigger {
    PostLink,
    PostActivate,
    OnRemove,
}
