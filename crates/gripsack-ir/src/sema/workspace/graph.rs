//! The admitted typed graph projection (plan/0052 §2.2 edge roles): one
//! walk over the decoded workspace collects every typed catalog edge —
//! production, build input, runtime, task prerequisite, validation,
//! retention — with its role, reference-site span and payload (artifact
//! selector, package command name), plus command-context violations
//! found in the same walk. Sibling modules judge the projection:
//! `refs` resolves edges against the catalog (E126, selector E130),
//! `context` judges command positions (E128), `cycles` rejects
//! dependency cycles over dependency roles only (E127). Validation and
//! retention edges are resolved and kind-checked but never feed the
//! build closure: a recipe gated by a check on the package it produces
//! is not a cycle. Ordered local lists (recipe steps) are intra-output
//! ordering — they never become catalog edges, so no `ordering`
//! variant exists here yet.

use crate::span::Span;
use crate::workspace::{Workspace, WorkspaceArg, WorkspaceCommand, WorkspaceOutput, WorkspacePath};

mod outputs;
use outputs::output_edges;

/// Outputs whose artifacts an `artifact` reference may address
/// (mirrors the TS emitter's `ARTIFACT_KINDS`).
const ARTIFACT_KINDS: &[&str] = &["recipe", "package"];
/// A check subject may name any admitted output kind.
const SUBJECT_KINDS: &[&str] = &[
    "recipe",
    "package",
    "environment",
    "task",
    "schedule",
    "check",
    "image",
    "profile",
    "hook",
];
const RECIPE: &[&str] = &["recipe"];
const PACKAGE: &[&str] = &["package"];
const ENVIRONMENT: &[&str] = &["environment"];
const TASK: &[&str] = &["task"];
const SCHEDULE: &[&str] = &["schedule"];
const CHECK: &[&str] = &["check"];
const HOOK: &[&str] = &["hook"];

/// The role an edge plays (0052 §2.2 role enum). The role decides
/// whether the edge feeds the build closure and cycle detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EdgeRole {
    /// package producer → recipe.
    Production,
    /// recipe step tool/artifact references — build-time only.
    BuildInput,
    /// Required at consumption: package runtime closure, environment
    /// and image package selections, task environment and tool
    /// references, check/hook tool references, profile artifact files.
    Runtime,
    /// task prerequisites — verified success within one invocation.
    TaskPrereq,
    /// Publication gates, invocation postconditions, check subjects —
    /// required gates, never prunable dead work, never build closure.
    Validation,
    /// Profile/schedule consumer wiring — retained roots that may
    /// legitimately close a loop, never build closure.
    Retention,
}

impl EdgeRole {
    /// Dependency edges feed cycle detection (E127); validation and
    /// retention edges are checked for existence and kind but can
    /// legitimately close a loop.
    pub(super) fn is_dependency(self) -> bool {
        matches!(
            self,
            Self::Production | Self::BuildInput | Self::Runtime | Self::TaskPrereq
        )
    }
}

/// How the two ends' platforms bind across an edge (0052 §2.2:
/// incompatible targets/layouts are admission rejections). Matching is
/// conservative v4 identity — exact os/arch/ABI/minimum-OS equality,
/// never a host-compatibility assumption for cross-target builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TargetBinding {
    /// No platform relationship.
    None,
    /// package producer ↔ recipe: targets must agree exactly.
    Producer,
    /// environment/image package selection: targets must agree exactly
    /// and a `fixed_prefix` package cannot be selected — the v4 wire
    /// has no consumer prefix slot to place it under.
    Selection,
}
/// A diagnostic relation labels the field (and optional environment
/// key) without allocating for every successful graph edge.
#[derive(Clone, Copy)]
pub(super) struct Relation<'a> {
    field: &'static str,
    key: Option<&'a str>,
}

impl<'a> Relation<'a> {
    fn field(field: &'static str) -> Self {
        Self { field, key: None }
    }

    fn variable(field: &'static str, key: &'a str) -> Self {
        Self {
            field,
            key: Some(key),
        }
    }
}

impl std::fmt::Display for Relation<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.key {
            Some(key) => write!(formatter, "{} `{key}`", self.field),
            None => formatter.write_str(self.field),
        }
    }
}

/// One typed catalog edge collected from the decoded IR.
pub(super) struct Edge<'a> {
    /// The referencing output.
    pub from: &'a WorkspaceOutput,
    /// The referenced catalog name.
    pub to: &'a str,
    pub role: EdgeRole,
    /// Admitted target kinds for this reference.
    pub expected: &'static [&'static str],
    /// Human-readable field and optional environment variable key.
    pub relation: Relation<'a>,
    /// The reference site: the enclosing command, file or output span.
    pub at: &'a Span,
    /// Artifact slots carry a selector to normalize (E130).
    pub selector: Option<&'a str>,
    /// `package_command` slots carry the invoked command name (E126).
    pub command: Option<&'a str>,
    pub binding: TargetBinding,
}

/// A command-context violation found while collecting edges (E128;
/// judged by the `context` module).
pub(super) enum ContextViolation<'a> {
    /// `package_command` as an environment *value* — values are data
    /// (literal or artifact), never command invocations.
    EnvCommand {
        relation: Relation<'a>,
        at: &'a Span,
    },
    /// A run_bash interpreter that is not a pinned `package_command` —
    /// ambient host discovery, which v4 never admits.
    AmbientInterpreter { at: &'a Span },
}

/// The admitted typed projection of one workspace.
pub(super) struct Projection<'a> {
    pub edges: Vec<Edge<'a>>,
    pub violations: Vec<ContextViolation<'a>>,
}

pub(super) fn collect(workspace: &Workspace) -> Projection<'_> {
    let mut projection = Projection {
        edges: Vec::new(),
        violations: Vec::new(),
    };
    for output in &workspace.outputs {
        output_edges(output, &mut projection);
    }
    projection
}

/// An edge naming outputs from a list field (deps, checks, packages…).
fn names_edges<'a>(
    from: &'a WorkspaceOutput,
    names: impl Iterator<Item = &'a String>,
    expected: &'static [&'static str],
    role: EdgeRole,
    relation: Relation<'a>,
    binding: TargetBinding,
    projection: &mut Projection<'a>,
) {
    for to in names {
        projection.edges.push(Edge {
            from,
            to,
            role,
            expected,
            relation,
            at: from.span(),
            selector: None,
            command: None,
            binding,
        });
    }
}

/// An artifact or package_command reference inside a command position
/// (exec argv slot or run_bash interpreter pin).
fn command_arg_edge<'a>(
    arg: &'a WorkspaceArg,
    from: &'a WorkspaceOutput,
    role: EdgeRole,
    relation: Relation<'a>,
    at: &'a Span,
    projection: &mut Projection<'a>,
) {
    match arg {
        WorkspaceArg::Literal { .. } => {}
        WorkspaceArg::Artifact { output, selector } => projection.edges.push(Edge {
            from,
            to: output,
            role,
            expected: ARTIFACT_KINDS,
            relation,
            at,
            selector: Some(selector),
            command: None,
            binding: TargetBinding::None,
        }),
        WorkspaceArg::PackageCommand { package, command } => projection.edges.push(Edge {
            from,
            to: package,
            role,
            expected: PACKAGE,
            relation,
            at,
            selector: None,
            command: Some(command),
            binding: TargetBinding::None,
        }),
    }
}

/// An environment *value*: data only. Artifact references are edges;
/// a package_command here is a context violation (E128), not an edge.
fn env_value<'a>(
    arg: &'a WorkspaceArg,
    from: &'a WorkspaceOutput,
    role: EdgeRole,
    relation: Relation<'a>,
    at: &'a Span,
    projection: &mut Projection<'a>,
) {
    match arg {
        WorkspaceArg::Literal { .. } => {}
        WorkspaceArg::Artifact { .. } => {
            command_arg_edge(arg, from, role, relation, at, projection);
        }
        WorkspaceArg::PackageCommand { .. } => {
            projection
                .violations
                .push(ContextViolation::EnvCommand { relation, at });
        }
    }
}

fn path_edge<'a>(
    path: &'a WorkspacePath,
    from: &'a WorkspaceOutput,
    role: EdgeRole,
    at: &'a Span,
    projection: &mut Projection<'a>,
) {
    if let WorkspacePath::Artifact { output, selector } = path {
        projection.edges.push(Edge {
            from,
            to: output,
            role,
            expected: ARTIFACT_KINDS,
            relation: Relation::field("command working directory"),
            at,
            selector: Some(selector),
            command: None,
            binding: TargetBinding::None,
        });
    }
}

/// One command body (exec or run_bash) under its enclosing output's
/// edge role.
fn command_edges<'a>(
    command: &'a WorkspaceCommand,
    from: &'a WorkspaceOutput,
    role: EdgeRole,
    projection: &mut Projection<'a>,
) {
    let span = command.span();
    let env_edges = |env: &'a std::collections::BTreeMap<String, WorkspaceArg>,
                     projection: &mut Projection<'a>| {
        for (variable, arg) in env {
            env_value(
                arg,
                from,
                role,
                Relation::variable("command environment variable", variable),
                span,
                projection,
            );
        }
    };
    match command {
        WorkspaceCommand::Exec { argv, env, cwd, .. } => {
            for arg in argv {
                command_arg_edge(
                    arg,
                    from,
                    role,
                    Relation::field("command argument"),
                    span,
                    projection,
                );
            }
            env_edges(env, projection);
            if let Some(cwd) = cwd {
                path_edge(cwd, from, role, span, projection);
            }
        }
        WorkspaceCommand::RunBash {
            interpreter,
            env,
            cwd,
            ..
        } => {
            match interpreter {
                WorkspaceArg::PackageCommand { .. } => command_arg_edge(
                    interpreter,
                    from,
                    role,
                    Relation::field("run_bash interpreter pin"),
                    span,
                    projection,
                ),
                _ => projection
                    .violations
                    .push(ContextViolation::AmbientInterpreter { at: span }),
            }
            env_edges(env, projection);
            if let Some(cwd) = cwd {
                path_edge(cwd, from, role, span, projection);
            }
        }
    }
}
