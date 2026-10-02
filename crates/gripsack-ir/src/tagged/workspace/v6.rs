use super::{WorkspaceWireVersion, check_tagged_at, raw_node_span};
use crate::{Diagnostic, Span, codes};
use serde_json::Value;

/// The v6 acquisition source union (A3): fetch (shared grammar minus
/// the legacy brew/pixi spellings the Conda lanes supersede), coherent
/// Conda environments and explicit Pixi lock imports.
fn acquisition_source_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "fetch" => &["kind", "fetch", "span"],
        "conda_environment" => &["kind", "channels", "packages", "platforms", "span"],
        "pixi_lock" => &["kind", "manifest", "lock", "environment", "span"],
        _ => return None,
    })
}
fn acquisition_fetch_fields(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "brew" | "pixi" => None,
        _ => super::allowed_fetch_fields(kind),
    }
}
/// Admit one v6 acquisition source node (recipe `source` or provider
/// `producer.provider`): the outer union, then the nested fetch spec.
pub(super) fn acquisition_source(
    node: &Value,
    path: &str,
    span: Option<&Span>,
    out: &mut Vec<Diagnostic>,
) {
    check_tagged_at(node, path, acquisition_source_fields, span, out);
    let Some(object) = node.as_object() else {
        return;
    };
    if object.get("kind").and_then(Value::as_str) == Some("fetch")
        && let Some(fetch) = object.get("fetch")
    {
        check_tagged_at(fetch, path, acquisition_fetch_fields, span, out);
    }
}
pub(super) fn layout_fields(kind: &str) -> Option<&'static [&'static str]> {
    if kind == "prefix_materialized" {
        Some(&["kind"])
    } else {
        super::allowed_workspace_layout_fields(kind)
    }
}
pub(super) fn output_fields(kind: &str) -> Option<&'static [&'static str]> {
    if kind == "task" {
        Some(&[
            "kind",
            "name",
            "span",
            "steps",
            "context",
            "deps",
            "environment",
            "checks",
            "mutation_locks",
        ])
    } else if kind == "image" {
        Some(&[
            "kind",
            "name",
            "span",
            "packages",
            "target",
            "base",
            "destinations",
            "config",
        ])
    } else {
        super::allowed_workspace_output_fields(kind)
    }
}
pub(super) fn execution_fields(kind: &str) -> Option<&'static [&'static str]> {
    if kind == "isolated_linux" {
        Some(&["kind", "worker", "platform", "toolchain"])
    } else {
        super::allowed_workspace_execution_fields(kind)
    }
}
pub(super) fn command_fields(kind: &str) -> Option<&'static [&'static str]> {
    if kind == "run_bash" {
        Some(&[
            "kind",
            "span",
            "interpreter",
            "options",
            "body",
            "env",
            "cwd",
            "line_map",
        ])
    } else {
        super::allowed_workspace_command_fields(kind)
    }
}
pub(super) fn argument_fields(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "package_command" => Some(&["kind", "package", "command", "sha256"]),
        "input" => Some(&["kind", "input"]),
        "source" | "output" => Some(&["kind", "selector"]),
        _ => super::allowed_workspace_arg_fields(kind),
    }
}
pub(super) fn path_fields(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "source" | "output" => Some(&["kind", "selector"]),
        _ => super::allowed_workspace_path_fields(kind),
    }
}
pub(super) fn source_fields(kind: &str) -> Option<&'static [&'static str]> {
    if kind == "tree" {
        Some(&["kind", "output", "include", "exclude"])
    } else {
        super::allowed_workspace_source_fields(kind)
    }
}
pub(super) fn content_fields(kind: &str) -> Option<&'static [&'static str]> {
    if kind == "template" {
        Some(&["kind", "template", "variables", "result_digest"])
    } else {
        super::allowed_workspace_content_fields(kind)
    }
}
fn input_fields(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "repo_file" => Some(&["kind", "path"]),
        "repo_directory" => Some(&["kind", "path", "include", "exclude"]),
        _ => None,
    }
}
fn action_fields(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "ensure_artifact" => Some(&["kind", "output", "span"]),
        _ => None,
    }
}
fn context_fields(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "host" => Some(&["kind", "mutable_paths"]),
        _ => None,
    }
}
pub(super) fn inputs(value: &Value, out: &mut Vec<Diagnostic>) {
    let Some(workspace) = value.get("workspace") else {
        return;
    };
    let workspace_span = raw_node_span(workspace);
    if let Some(inputs) = workspace.get("inputs").and_then(Value::as_array) {
        for input in inputs {
            let span = raw_node_span(input).or_else(|| workspace_span.clone());
            if let Some(origin) = input.get("origin") {
                check_tagged_at(origin, "workspace input", input_fields, span.as_ref(), out);
            }
        }
    }
}
pub(super) fn steps(output: &Value, path: &str, span: Option<&Span>, out: &mut Vec<Diagnostic>) {
    if let Some(context) = output.get("context") {
        check_tagged_at(context, path, context_fields, span, out);
    }
    if let Some(steps) = output.get("steps").and_then(Value::as_array) {
        for step in steps {
            let valid = step.as_object().is_some_and(|object| {
                object.len() == 1
                    && (object.contains_key("command") || object.contains_key("action"))
            });
            if !valid {
                out.push(
                    Diagnostic::error(
                        codes::MALFORMED,
                        "a local step must contain exactly one command or action",
                    )
                    .with_label(span.cloned(), "step declared here"),
                );
            }
            if let Some(command) = step.get("command") {
                super::check_workspace_command(
                    command,
                    path,
                    span,
                    WorkspaceWireVersion::CurrentV6,
                    out,
                );
            }
            if let Some(action) = step.get("action") {
                let action_span = raw_node_span(action).or_else(|| span.cloned());
                check_tagged_at(action, path, action_fields, action_span.as_ref(), out);
            }
        }
    }
}
