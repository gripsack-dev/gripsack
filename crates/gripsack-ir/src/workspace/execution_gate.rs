//! Read-only workspace admission is not an execution entitlement.
//! Both the CLI and direct executor entrypoints ask this single core
//! gate before opening a home, a lockfile, a worker or a scheduler.
//! Label the first declared unavailable output rather than an invented
//! host fallback or an undifferentiated workspace root.

use super::{RecipeExecution, WorkspaceOutput};
use crate::diagnostic::{Diagnostic, codes};
use crate::model::Ir;
use crate::span::Span;

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

fn current_output<'a>(ir: &'a Ir) -> Option<DeclaredCapability<'a>> {
    let workspace = ir.workspace.as_ref()?;
    let Some(output) = workspace.outputs.first() else {
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

pub(crate) fn execution_error(ir: &Ir, operation: &str) -> Option<Diagnostic> {
    let declaration = current_output(ir).or_else(|| historical_output(ir))?;
    let capability = declaration.capability;
    let help = match capability {
        UnavailableCapability::HistoricalV4 => {
            "historical v4 workspaces stay read-only; migrate authoring to the current v5 wire. grip check still validates the saved workspace"
        }
        _ => {
            "grip check validates named outputs without starting a builder or scheduler; workspace execution remains unavailable in this binary"
        }
    };
    Some(
        Diagnostic::error(
            codes::WORKSPACE_EXEC_UNAVAILABLE,
            format!(
                "{operation} cannot execute output `{}`: {} belongs to {}; no workspace executor is available",
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
