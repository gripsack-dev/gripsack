//! The v4 workspace tagged-field walk (schema/ir/v4.json, plan/0052
//! §2.1–2.2): every tagged node in the workspace catalog — outputs,
//! commands and their arguments, the run_bash interpreter pin, cwd
//! paths, file origin/content/destination policies, calendar triggers,
//! the producer union, and the nested v3 fetch specs under
//! `source.fetch` / `producer.provider.fetch` — is closed against a
//! per-kind allowlist before serde can drop anything. Plain structs
//! (workspace, platform, file, span) are closed by
//! `deny_unknown_fields` instead.
//!
//! Rejections label the NEAREST declaring node's span, read from the
//! raw JSON: an output-level extra labels the output, a command or
//! argument extra labels the command (else the owning output), a fetch
//! extra labels its workspaceFetch source/provider span. v3 diagnostics
//! never pass through here — the v3 module walk stays in `tagged.rs`
//! with its span-free messages.

use super::{allowed_fetch_fields, check_tagged_at, raw_node_span};
use crate::diagnostic::Diagnostic;

/// Workspace output kinds → allowed keys (the tag plus the variant's
/// fields; schema/ir/v4.json `workspaceOutput` oneOf).
fn allowed_workspace_output_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "recipe" => &[
            "kind",
            "name",
            "span",
            "source",
            "execution",
            "output_kind",
            "target",
            "steps",
            "checks",
        ],
        "package" => &[
            "kind", "name", "span", "producer", "commands", "runtime", "target", "layout",
        ],
        "environment" => &["kind", "name", "span", "packages", "target", "env"],
        "task" => &[
            "kind",
            "name",
            "span",
            "run",
            "deps",
            "environment",
            "checks",
        ],
        "schedule" => &["kind", "name", "span", "task", "trigger", "scope"],
        "check" => &["kind", "name", "span", "run", "subject"],
        "image" => &["kind", "name", "span", "packages", "target"],
        "profile" => &[
            "kind",
            "name",
            "span",
            "files",
            "environment",
            "schedules",
            "hooks",
        ],
        "hook" => &["kind", "name", "span", "run", "trigger"],
        _ => return None,
    })
}

fn allowed_workspace_command_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "exec" => &["kind", "span", "argv", "env", "cwd"],
        "run_bash" => &[
            "kind",
            "span",
            "interpreter",
            "body",
            "env",
            "cwd",
            "line_map",
        ],
        _ => return None,
    })
}

fn allowed_workspace_arg_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "literal" => &["kind", "value"],
        "artifact" => &["kind", "output", "selector"],
        "package_command" => &["kind", "package", "command"],
        _ => return None,
    })
}

fn allowed_workspace_path_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "literal" => &["kind", "value"],
        "artifact" => &["kind", "output", "selector"],
        "host" => &["kind", "path"],
        _ => return None,
    })
}

fn allowed_workspace_source_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "repo_file" => &["kind", "path"],
        "artifact_file" => &["kind", "output", "selector"],
        _ => return None,
    })
}

fn allowed_workspace_content_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "identity" => &["kind"],
        "literal" => &["kind", "text"],
        "template" => &["kind", "template", "variables"],
        _ => return None,
    })
}

fn allowed_workspace_destination_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "symlink" | "tracked_copy" => &["kind", "path"],
        "managed_block" => &["kind", "path", "marker"],
        _ => return None,
    })
}

fn allowed_workspace_calendar_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "daily" => &["kind", "time"],
        "weekly" => &["kind", "weekday", "time"],
        _ => return None,
    })
}

fn allowed_workspace_producer_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "recipe" => &["kind", "recipe"],
        "provider" => &["kind", "provider"],
        _ => return None,
    })
}

/// A workspace command node and every tagged node nested in it: argv
/// and env arguments, the run_bash interpreter pin, the cwd path.
/// Rejections label the command's own span, falling back to the
/// enclosing output's declaration span.
fn check_workspace_command(
    node: &serde_json::Value,
    path: &str,
    output_span: Option<&crate::Span>,
    out: &mut Vec<Diagnostic>,
) {
    let command_span = raw_node_span(node).or_else(|| output_span.cloned());
    let span = command_span.as_ref();
    check_tagged_at(node, path, allowed_workspace_command_fields, span, out);
    let Some(obj) = node.as_object() else { return };
    if let Some(argv) = obj.get("argv").and_then(|a| a.as_array()) {
        for arg in argv {
            check_tagged_at(arg, path, allowed_workspace_arg_fields, span, out);
        }
    }
    if let Some(interpreter) = obj.get("interpreter") {
        check_tagged_at(interpreter, path, allowed_workspace_arg_fields, span, out);
    }
    if let Some(env) = obj.get("env").and_then(|e| e.as_object()) {
        for arg in env.values() {
            check_tagged_at(arg, path, allowed_workspace_arg_fields, span, out);
        }
    }
    if let Some(cwd) = obj.get("cwd") {
        check_tagged_at(cwd, path, allowed_workspace_path_fields, span, out);
    }
}

/// Walk the v4 workspace catalog (`/workspace/outputs/*`).
pub(super) fn check(value: &serde_json::Value, out: &mut Vec<Diagnostic>) {
    let Some(outputs) = value
        .get("workspace")
        .and_then(|w| w.get("outputs"))
        .and_then(|o| o.as_array())
    else {
        return;
    };
    for output in outputs {
        let name = output
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("?")
            .to_string();
        let kind = output
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or("")
            .to_string();
        let path = format!("workspace output {name:?} ({kind})");
        let output_span = raw_node_span(output);
        check_tagged_at(
            output,
            &path,
            allowed_workspace_output_fields,
            output_span.as_ref(),
            out,
        );
        match kind.as_str() {
            "recipe" => {
                if let Some(source) = output.get("source") {
                    // The workspaceFetch wrapper's own span, else the recipe's.
                    let source_span = raw_node_span(source).or_else(|| output_span.clone());
                    if let Some(fetch) = source.get("fetch") {
                        check_tagged_at(
                            fetch,
                            &path,
                            allowed_fetch_fields,
                            source_span.as_ref(),
                            out,
                        );
                    }
                }
                if let Some(steps) = output.get("steps").and_then(|s| s.as_array()) {
                    for command in steps {
                        check_workspace_command(command, &path, output_span.as_ref(), out);
                    }
                }
            }
            "package" => {
                if let Some(producer) = output.get("producer") {
                    check_tagged_at(
                        producer,
                        &path,
                        allowed_workspace_producer_fields,
                        output_span.as_ref(),
                        out,
                    );
                    // /workspace/outputs/*/producer/provider/fetch gets
                    // the same admission as a recipe's source fetch —
                    // serde cannot close the nested tagged union.
                    if let Some(provider) = producer.get("provider") {
                        let provider_span = raw_node_span(provider).or_else(|| output_span.clone());
                        if let Some(fetch) = provider.get("fetch") {
                            check_tagged_at(
                                fetch,
                                &path,
                                allowed_fetch_fields,
                                provider_span.as_ref(),
                                out,
                            );
                        }
                    }
                }
            }
            "environment" => {
                if let Some(env) = output.get("env").and_then(|e| e.as_object()) {
                    for arg in env.values() {
                        check_tagged_at(
                            arg,
                            &path,
                            allowed_workspace_arg_fields,
                            output_span.as_ref(),
                            out,
                        );
                    }
                }
            }
            "task" | "check" | "hook" => {
                if let Some(run) = output.get("run") {
                    check_workspace_command(run, &path, output_span.as_ref(), out);
                }
            }
            "schedule" => {
                if let Some(trigger) = output.get("trigger") {
                    check_tagged_at(
                        trigger,
                        &path,
                        allowed_workspace_calendar_fields,
                        output_span.as_ref(),
                        out,
                    );
                }
            }
            "profile" => {
                if let Some(files) = output.get("files").and_then(|f| f.as_array()) {
                    for file in files {
                        // The file declaration's own span, else the profile's.
                        let file_span = raw_node_span(file).or_else(|| output_span.clone());
                        if let Some(source) = file.get("source") {
                            check_tagged_at(
                                source,
                                &path,
                                allowed_workspace_source_fields,
                                file_span.as_ref(),
                                out,
                            );
                        }
                        if let Some(content) = file.get("content") {
                            check_tagged_at(
                                content,
                                &path,
                                allowed_workspace_content_fields,
                                file_span.as_ref(),
                                out,
                            );
                        }
                        if let Some(destination) = file.get("destination") {
                            check_tagged_at(
                                destination,
                                &path,
                                allowed_workspace_destination_fields,
                                file_span.as_ref(),
                                out,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
