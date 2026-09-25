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

pub(super) use gripsack_policy::graph::roles::GraphRole as EdgeRole;

mod outputs;
use outputs::output_edges;

/// Outputs whose artifacts an `artifact` reference may address
/// (mirrors the TS emitter's `ARTIFACT_KINDS`).
pub(super) const ARTIFACT_KINDS: &[&str] = &["recipe", "package"];
/// A check subject may name any admitted output kind.
pub(super) const SUBJECT_KINDS: &[&str] = &[
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
pub(super) const RECIPE: &[&str] = &["recipe"];
pub(super) const PACKAGE: &[&str] = &["package"];
pub(super) const ENVIRONMENT: &[&str] = &["environment"];
pub(super) const TASK: &[&str] = &["task"];
pub(super) const SCHEDULE: &[&str] = &["schedule"];
pub(super) const CHECK: &[&str] = &["check"];
pub(super) const HOOK: &[&str] = &["hook"];

/// Target relationships along named-output edges (0052 §2.2).
/// Providers must match OS/arch/ABI and not require a newer OS floor
/// than their consumers. Checking-host facts never select a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TargetBinding {
    /// No platform relationship.
    None,
    /// Package producer recipe requirements must fit package target.
    Producer,
    /// Package selections compare target floors; prefix-bound packages
    /// need a matching environment prefix (images remain unavailable).
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
    /// ambient host discovery, forbidden by workspace admission.
    AmbientInterpreter { at: &'a Span },
}

/// Sequencing is local to a recipe; step indexes are not output names
/// or independent cache identities. Intra-recipe edges never enter
/// the catalog's build/runtime closure.
pub(super) struct LocalOrder<'a> {
    pub recipe: &'a str,
    pub before: usize,
    pub after: usize,
    pub at: &'a Span,
    pub role: EdgeRole,
}

/// The admitted typed projection of one workspace.
pub(super) struct Projection<'a> {
    pub edges: Vec<Edge<'a>>,
    pub ordering: Vec<LocalOrder<'a>>,
    pub violations: Vec<ContextViolation<'a>>,
}

pub(super) fn collect(workspace: &Workspace) -> Projection<'_> {
    let mut projection = Projection {
        edges: Vec::new(),
        ordering: Vec::new(),
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
