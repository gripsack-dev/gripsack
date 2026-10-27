//! Traverse the single current workspace grammar before typed decoding.
//! Plain structs deny unknown fields themselves; tagged unions use field rules.

use super::{allowed_fetch_fields, check_tagged_at, raw_node_span};
use crate::{Diagnostic, Span};
use serde_json::Value;

mod commands;
mod fields;

fn acquisition(node: &Value, path: &str, owner: Option<&Span>, out: &mut Vec<Diagnostic>) {
    let span = raw_node_span(node);
    let span = span.as_ref().or(owner);
    check_tagged_at(node, path, fields::acquisition, span, out);
    if node.get("kind").and_then(Value::as_str) == Some("fetch")
        && let Some(fetch) = node.get("fetch")
    {
        check_tagged_at(fetch, path, allowed_fetch_fields, span, out);
    }
}

pub(super) fn check(value: &Value, out: &mut Vec<Diagnostic>) {
    let Some(workspace) = value.get("workspace") else {
        return;
    };
    let workspace_span = raw_node_span(workspace);
    if let Some(inputs) = workspace.get("inputs").and_then(Value::as_array) {
        for input in inputs {
            let span = raw_node_span(input);
            if let Some(origin) = input.get("origin") {
                check_tagged_at(
                    origin,
                    "workspace input",
                    fields::input,
                    span.as_ref().or(workspace_span.as_ref()),
                    out,
                );
            }
        }
    }
    let Some(outputs) = workspace.get("outputs").and_then(Value::as_array) else {
        return;
    };
    for output in outputs {
        let name = output.get("name").and_then(Value::as_str).unwrap_or("?");
        let kind = output.get("kind").and_then(Value::as_str).unwrap_or("");
        let path = format!("workspace output {name:?} ({kind})");
        let output_span = raw_node_span(output);
        let span = output_span.as_ref();
        check_tagged_at(output, &path, fields::output, span, out);
        match kind {
            "recipe" => {
                if let Some(execution) = output.get("execution") {
                    check_tagged_at(execution, &path, fields::execution, span, out);
                }
                if let Some(source) = output.get("source") {
                    acquisition(source, &path, span, out);
                }
                commands::steps(output, &path, span, out);
            }
            "package" => {
                if let Some(root) = output.get("archive_root") {
                    check_tagged_at(root, &path, fields::archive_root, span, out);
                }
                if let Some(layout) = output.get("layout") {
                    check_tagged_at(layout, &path, fields::layout, span, out);
                }
                if let Some(producer) = output.get("producer") {
                    check_tagged_at(producer, &path, fields::producer, span, out);
                    if let Some(provider) = producer.get("provider") {
                        acquisition(provider, &path, span, out);
                    }
                }
            }
            "image" => {
                if let Some(arguments) = output
                    .get("config")
                    .and_then(|config| config.get("entrypoint"))
                    .and_then(Value::as_array)
                {
                    for argument in arguments {
                        check_tagged_at(argument, &path, fields::argument, span, out);
                    }
                }
            }
            "environment" => {
                if let Some(env) = output.get("env").and_then(Value::as_object) {
                    for argument in env.values() {
                        check_tagged_at(argument, &path, fields::argument, span, out);
                    }
                }
            }
            "task" => commands::steps(output, &path, span, out),
            "check" | "hook" => {
                if let Some(run) = output.get("run") {
                    commands::check(run, &path, span, out);
                }
            }
            "schedule" => {
                if let Some(trigger) = output.get("trigger") {
                    check_tagged_at(trigger, &path, fields::calendar, span, out);
                }
            }
            "profile" => {
                if let Some(launchers) = output.get("launchers").and_then(Value::as_object) {
                    for key in launchers.keys() {
                        if !matches!(key.as_str(), "directory" | "commands") {
                            out.push(
                                Diagnostic::error(
                                    crate::codes::MALFORMED,
                                    format!("unknown field `{key}` in profile launchers ({path})"),
                                )
                                .with_label(span.cloned(), "declared here"),
                            );
                        }
                    }
                }
                if let Some(files) = output.get("files").and_then(Value::as_array) {
                    for file in files {
                        let file_span = raw_node_span(file);
                        let span = file_span.as_ref().or(span);
                        if let Some(source) = file.get("source") {
                            check_tagged_at(source, &path, fields::source, span, out);
                        }
                        if let Some(content) = file.get("content") {
                            check_tagged_at(content, &path, fields::content, span, out);
                        }
                        if let Some(destination) = file.get("destination") {
                            check_tagged_at(destination, &path, fields::destination, span, out);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
