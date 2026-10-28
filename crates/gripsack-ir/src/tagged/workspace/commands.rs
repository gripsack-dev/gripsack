//! Commands and ordered local actions inherit their declaring source span.

use super::{check_tagged_at, fields, raw_node_span};
use crate::{Diagnostic, Span, codes};
use serde_json::Value;

pub(super) fn check(node: &Value, path: &str, owner: Option<&Span>, out: &mut Vec<Diagnostic>) {
    let span = raw_node_span(node);
    let span = span.as_ref().or(owner);
    check_tagged_at(node, path, fields::command, span, out);
    if let Some(argv) = node.get("argv").and_then(Value::as_array) {
        for argument in argv {
            check_tagged_at(argument, path, fields::argument, span, out);
        }
    }
    if let Some(interpreter) = node.get("interpreter") {
        check_tagged_at(interpreter, path, fields::argument, span, out);
    }
    if let Some(env) = node.get("env").and_then(Value::as_object) {
        for argument in env.values() {
            check_tagged_at(argument, path, fields::argument, span, out);
        }
    }
    if let Some(cwd) = node.get("cwd") {
        check_tagged_at(cwd, path, fields::path, span, out);
    }
}

pub(super) fn steps(output: &Value, path: &str, span: Option<&Span>, out: &mut Vec<Diagnostic>) {
    if let Some(context) = output.get("context") {
        check_tagged_at(context, path, fields::context, span, out);
    }
    let Some(steps) = output.get("steps").and_then(Value::as_array) else {
        return;
    };
    for step in steps {
        let valid = step.as_object().is_some_and(|object| {
            object.len() == 1 && (object.contains_key("command") || object.contains_key("action"))
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
            check(command, path, span, out);
        }
        if let Some(action) = step.get("action") {
            let action_span = raw_node_span(action);
            check_tagged_at(
                action,
                path,
                fields::action,
                action_span.as_ref().or(span),
                out,
            );
        }
    }
}
