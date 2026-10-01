//! The v6 named-output catalog (0052 §2.1, v6-design §§2.1–2.3): the
//! workspace envelope entry, the nine output types, plus inputs,
//! mutation locks and the v6 execution/toolchain variants. Layout,
//! target, calendar and fetch types are SHARED with the frozen v5
//! module (`crate::workspace`), never duplicated. `outputs` names form
//! the single declaration namespace; `inputs` are a second namespace —
//! a collision in either reports both declaration spans (sema E125).

use super::command::{TaskContext, WorkspaceArg, WorkspaceCommand, WorkspaceStep};
use super::file::WorkspaceFile;
use super::image::ImageOutput;
use super::source::WorkspaceSourceV6;
use crate::span::Span;
use crate::workspace::{
    HostAccess, InstallPrefix, LinuxWorker, RecipeOutputKind, WorkspaceCalendar, WorkspacePlatform,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Current v6 workspace entry: one catalog of named outputs, plus the
/// input and mutation-lock declarations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceV6 {
    /// Where the workspace itself was declared. Mandatory in v6.
    pub span: Span,
    /// Optional display name; never a producer or worktree identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The named-output catalog (schema: minItems 1, sema E130).
    pub outputs: Vec<WorkspaceOutput>,
    /// Typed workspace inputs — a separate namespace from outputs
    /// (sema E125 names both spans on duplicates).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<WorkspaceInput>,
    /// Workspace-level mutation-lock declarations; use sites reference
    /// them by (scope, key) (sema E126/E130).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutation_locks: Vec<WorkspaceMutationLock>,
}

/// One declared workspace input (v6-design §2.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceInput {
    pub name: String,
    pub span: Span,
    pub origin: InputOrigin,
}

/// Captured inputs are distinct from the live writable checkout of a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputOrigin {
    RepoFile {
        path: String,
    },
    RepoDirectory {
        path: String,
        include: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        exclude: Vec<String>,
    },
}

/// A declared mutation lock (v6-design §2.1): `key` is 1..=128 bytes
/// of printable non-whitespace ASCII (sema E130).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceMutationLock {
    pub scope: MutationLockScope,
    pub key: String,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationLockScope {
    User,
    Store,
}

/// One declared v6 workspace output. The `kind` tag is closed by the
/// tagged-field pass; the nine variants are the current v6 grammar.
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

/// A build description: source bytes plus an ordered local step list,
/// producing a file or tree artifact for one target platform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeOutput {
    pub name: String,
    pub span: Span,
    /// How to obtain the payload — the v6 acquisition grammar with its
    /// own declaration span.
    pub source: WorkspaceSourceV6,
    pub execution: RecipeExecution,
    pub output_kind: RecipeOutputKind,
    pub target: WorkspacePlatform,
    /// Ordered build steps (order is semantic).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<WorkspaceStep>,
    /// Publication gates — names of `check` outputs (sema E126).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<String>,
}

/// The v6 execution boundary [RESPELL: v5 `isolated_linux{worker}` is
/// superseded — v6 carries the platform and toolchain reference].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecipeExecution {
    Host {
        access: HostAccess,
    },
    IsolatedLinux {
        worker: LinuxWorker,
        platform: WorkspacePlatform,
        toolchain: ToolchainReference,
    },
}

/// A pinned toolchain image reference (v6-design §2.3): lexical form
/// `<name>@sha256:<64 lowercase hex>` is enforced by sema (E130); the
/// reader admits the string opaquely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolchainReference {
    pub reference: String,
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
    pub layout: PackageLayoutV6,
}

/// What produces a package's payload (plan/0052 §2.2: resolution and
/// acquisition stay separated — schema `$defs/workspaceProducer`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceProducer {
    /// Name of the `recipe` output that builds this package (sema E126).
    Recipe { recipe: String },
    /// A direct provider acquisition — the v6 source grammar with
    /// mandatory provenance. A provider-backed package needs no
    /// synthetic recipe output.
    Provider { provider: WorkspaceSourceV6 },
}

/// The v6 package layout (A3): the shared v5 pair stays with the frozen
/// reader; v6 adds prefix-materialized packages whose durable prefix is
/// derived from the frozen lock and store context at materialization
/// time (never an author-declared literal and never relocatable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PackageLayoutV6 {
    Relocatable,
    /// A fixed literal prefix declared for cross-target admission.
    FixedPrefix { prefix: InstallPrefix },
    /// Payload materialized at a derived durable prefix (Conda lanes):
    /// the exact prefix is receipt authority, not declaration text.
    PrefixMaterialized,
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

/// One invocable unit of work [RESPELL of v5 `run`: v6 tasks carry
/// `steps` and a required `context`]. `deps` are unordered prerequisites
/// verified within one invocation (E1 owns runtime); cycles are an
/// admission error (sema E127).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskOutput {
    pub name: String,
    pub span: Span,
    /// Ordered invocation steps (schema: minItems 1, sema E130).
    pub steps: Vec<WorkspaceStep>,
    /// Required in v6, closed to unconfined host plus declared mutable
    /// paths (v6-design §2.3).
    pub context: TaskContext,
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
    /// Use-site mutation locks; each must match a workspace-level
    /// declaration by (scope, key) (sema E126).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutation_locks: Vec<WorkspaceMutationLock>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span {
            file: "gripsack.ts".to_string(),
            line: 1,
            col: None,
        }
    }

    #[test]
    fn task_context_is_required() {
        let base = serde_json::json!({
            "kind": "task",
            "name": "t",
            "span": span(),
            "steps": [{"action": {"kind": "ensure_artifact", "output": "a", "span": span()}}]
        });
        assert!(serde_json::from_value::<WorkspaceOutput>(base.clone()).is_err());
        let mut with_context = base;
        with_context["context"] = serde_json::json!({"kind": "host", "mutable_paths": []});
        let output: WorkspaceOutput = serde_json::from_value(with_context).unwrap();
        assert_eq!(output.name(), "t");
        assert_eq!(output.kind(), "task");
        assert_eq!(output.span(), &span());
    }
}
