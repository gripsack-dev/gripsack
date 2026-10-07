//! Read-only workspace admission is not an execution entitlement.
//! Both the CLI and direct executor entrypoints ask this single core
//! gate before opening a home, a lockfile, a worker or a scheduler.
//! Label the first declared unavailable output rather than an invented
//! host fallback or an undifferentiated workspace root.

use super::{RecipeExecution, WorkspaceOutput, WorkspaceSource};
use crate::diagnostic::{Diagnostic, codes};
use crate::model::Ir;
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceOperation {
    Apply,
    Plan,
    Update,
    Build,
    Run,
    Shell,
    Task,
}

impl std::fmt::Display for WorkspaceOperation {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(match self {
            Self::Apply => "apply",
            Self::Plan => "plan",
            Self::Update => "update",
            Self::Build => "build",
            Self::Run => "run",
            Self::Shell => "shell",
            Self::Task => "task",
        })
    }
}

/// The executor milestone that owns one declared capability. This is
/// diagnostic classification only: it never authorizes an effect.
#[derive(Clone, Copy)]
enum UnavailableCapability {
    HostRecipe,
    IsolatedLinuxRecipe,
    Package,
    Environment,
    Task,
    TaskPrerequisites,
    Schedule,
    Check,
    Image,
    Profile,
    Hook,
    HistoricalV4,
    EmptyCatalog,
}

impl UnavailableCapability {
    fn description(self) -> &'static str {
        match self {
            Self::HostRecipe => "host recipe realization",
            Self::IsolatedLinuxRecipe => "isolated_linux recipe worker",
            Self::Package => "package realization",
            Self::Environment => "environment activation",
            Self::Task => "task invocation",
            Self::TaskPrerequisites => "task prerequisite execution",
            Self::Schedule => "schedule registration",
            Self::Check => "check execution",
            Self::Image => "image materialization",
            Self::Profile => "profile deployment",
            Self::Hook => "hook execution",
            Self::HistoricalV4 => "historical v4 workspace execution",
            Self::EmptyCatalog => "empty workspace catalog",
        }
    }

    fn owner(self) -> &'static str {
        match self {
            Self::HostRecipe | Self::Package | Self::Profile | Self::Hook | Self::EmptyCatalog => {
                "A2"
            }
            Self::IsolatedLinuxRecipe => "B2",
            Self::Environment | Self::Task => "A2-P",
            Self::TaskPrerequisites => "E1",
            Self::Schedule => "E2/E3",
            Self::Check => "A2/E1",
            Self::Image => "B4",
            Self::HistoricalV4 => "A5 migration",
        }
    }
}

struct DeclaredCapability<'a> {
    capability: UnavailableCapability,
    name: &'a str,
    span: &'a Span,
    workspace_span: &'a Span,
}

fn current_output<'a>(ir: &'a Ir, operation: WorkspaceOperation) -> Option<DeclaredCapability<'a>> {
    if let Some(workspace) = &ir.workspace_v6 {
        return current_v6_output(workspace, operation);
    }
    let workspace = ir.workspace.as_ref()?;
    let Some(output) = workspace
        .outputs
        .iter()
        .find(|output| !native_profile(output))
    else {
        if !workspace.outputs.is_empty() {
            return None;
        }
        return Some(DeclaredCapability {
            capability: UnavailableCapability::EmptyCatalog,
            name: "workspace",
            span: &workspace.span,
            workspace_span: &workspace.span,
        });
    };
    let capability = match output {
        WorkspaceOutput::Recipe(recipe) => match recipe.execution {
            RecipeExecution::Host { .. } => UnavailableCapability::HostRecipe,
            RecipeExecution::IsolatedLinux { .. } => UnavailableCapability::IsolatedLinuxRecipe,
        },
        WorkspaceOutput::Package(_) => UnavailableCapability::Package,
        WorkspaceOutput::Environment(_) => UnavailableCapability::Environment,
        WorkspaceOutput::Task(task) if !task.deps.is_empty() => {
            UnavailableCapability::TaskPrerequisites
        }
        WorkspaceOutput::Task(_) => UnavailableCapability::Task,
        WorkspaceOutput::Schedule(_) => UnavailableCapability::Schedule,
        WorkspaceOutput::Check(_) => UnavailableCapability::Check,
        WorkspaceOutput::Image(_) => UnavailableCapability::Image,
        WorkspaceOutput::Profile(_) => UnavailableCapability::Profile,
        WorkspaceOutput::Hook(_) => UnavailableCapability::Hook,
    };
    Some(DeclaredCapability {
        capability,
        name: output.name(),
        span: output.span(),
        workspace_span: &workspace.span,
    })
}

fn native_profile(output: &WorkspaceOutput) -> bool {
    matches!(output, WorkspaceOutput::Profile(profile)
        if profile.environment.is_none() && profile.hooks.is_empty() && profile.schedules.is_empty()
        && profile.files.iter().all(|file| !matches!(file.source, Some(WorkspaceSource::ArtifactFile { .. }))))
}

/// Profile deployability is operation-specific. `apply` realizes and
/// deploys environment selections and artifact/tree file origins through the
/// common realization service. `plan` previews the SAME profiles offline:
/// produced content renders as explicit deferred operations and no builder,
/// download, solve or activation runs. Hooks share durable post-flip activation;
/// schedules and staged file checks keep their own executor lanes.
fn current_v6_output(
    workspace: &crate::workspace_v6::WorkspaceV6,
    operation: WorkspaceOperation,
) -> Option<DeclaredCapability<'_>> {
    use crate::workspace_v6::{RecipeExecution, WorkspaceOutput};
    let deployable = |profile: &crate::workspace_v6::ProfileOutput| {
        profile.schedules.is_empty() && profile.files.iter().all(|file| file.checks.is_empty())
    };
    let mut first_non_profile: Option<&WorkspaceOutput> = None;
    let mut has_deployable_profile = false;
    for output in &workspace.outputs {
        match output {
            WorkspaceOutput::Profile(profile) if deployable(profile) => {
                has_deployable_profile = true;
            }
            other => {
                if first_non_profile.is_none() {
                    first_non_profile = Some(other);
                }
            }
        }
    }
    // A deployable profile deploys through apply and previews through plan;
    // declarations it does not reference stay inert and must not block it.
    if has_deployable_profile
        && matches!(
            operation,
            WorkspaceOperation::Apply | WorkspaceOperation::Plan
        )
    {
        return None;
    }
    let Some(output) = first_non_profile else {
        return workspace.outputs.is_empty().then_some(DeclaredCapability {
            capability: UnavailableCapability::EmptyCatalog,
            name: "workspace",
            span: &workspace.span,
            workspace_span: &workspace.span,
        });
    };
    let capability = match output {
        WorkspaceOutput::Recipe(recipe) => match recipe.execution {
            RecipeExecution::Host { .. } => UnavailableCapability::HostRecipe,
            RecipeExecution::IsolatedLinux { .. } => UnavailableCapability::IsolatedLinuxRecipe,
        },
        WorkspaceOutput::Package(_) => UnavailableCapability::Package,
        WorkspaceOutput::Environment(_) => UnavailableCapability::Environment,
        WorkspaceOutput::Task(task) if !task.deps.is_empty() => {
            UnavailableCapability::TaskPrerequisites
        }
        WorkspaceOutput::Task(_) => UnavailableCapability::Task,
        WorkspaceOutput::Schedule(_) => UnavailableCapability::Schedule,
        WorkspaceOutput::Check(_) => UnavailableCapability::Check,
        WorkspaceOutput::Image(_) => UnavailableCapability::Image,
        WorkspaceOutput::Profile(_) => UnavailableCapability::Profile,
        WorkspaceOutput::Hook(_) => UnavailableCapability::Hook,
    };
    Some(DeclaredCapability {
        capability,
        name: output.name(),
        span: output.span(),
        workspace_span: &workspace.span,
    })
}

fn historical_output<'a>(ir: &'a Ir) -> Option<DeclaredCapability<'a>> {
    let workspace = ir.workspace_v4.as_ref()?;
    let Some(output) = workspace.outputs.first() else {
        return Some(DeclaredCapability {
            capability: UnavailableCapability::HistoricalV4,
            name: "workspace",
            span: &workspace.span,
            workspace_span: &workspace.span,
        });
    };
    Some(DeclaredCapability {
        capability: UnavailableCapability::HistoricalV4,
        name: output.name(),
        span: output.span(),
        workspace_span: &workspace.span,
    })
}

pub(crate) fn execution_error(ir: &Ir, operation: WorkspaceOperation) -> Option<Diagnostic> {
    // These services admit the selected closure themselves. Catalog entries
    // that are not selected must not acquire effects or block another output.
    if ir.ir_version == crate::IR_VERSION
        && ir.workspace_v6.is_some()
        && ir.workspace.is_none()
        && ir.workspace_v4.is_none()
        && ir.modules.is_empty()
        && matches!(
            operation,
            WorkspaceOperation::Update
                | WorkspaceOperation::Build
                | WorkspaceOperation::Run
                | WorkspaceOperation::Shell
                | WorkspaceOperation::Task
        )
    {
        return None;
    }
    let declaration = current_output(ir, operation).or_else(|| historical_output(ir))?;
    let capability = declaration.capability;
    let help = match capability {
        UnavailableCapability::HistoricalV4 => {
            "historical v4 workspaces stay read-only; migrate authoring to the current v6 wire. grip check still validates the saved workspace"
        }
        _ => {
            "native profiles deploy captured files and realize artifact/environment inputs at apply; this output still requires its named executor capability"
        }
    };
    Some(
        Diagnostic::error(
            codes::WORKSPACE_EXEC_UNAVAILABLE,
            format!(
                "{operation} cannot execute output `{}`: {} belongs to {}; its executor capability is unavailable",
                declaration.name,
                capability.description(),
                capability.owner(),
            ),
        )
        .with_label(Some(declaration.span.clone()), "unavailable output declared here")
        .with_label(
            Some(declaration.workspace_span.clone()),
            "workspace declared here",
        )
        .with_help(help),
    )
}
