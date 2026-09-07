//! One-shot NDJSON adapter. The process supervisor owns all I/O and cleanup;
//! a protocol response suppresses callbacks, never outcome checks.

mod diagnostics;
#[cfg(test)]
mod tests;

use diagnostics::{from_plugin, host_diagnostic};
use gripsack_ir::{Diagnostic, Severity, Span};
use gripsack_process::{Control, Limits, StopReason};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const LINT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(serde::Serialize)]
struct LintRequest<'a> {
    op: &'static str,
    paths: &'a [PathBuf],
    tool_version: Option<&'a str>,
}

pub(crate) fn run_linter(
    exe: &Path,
    name: &str,
    paths: &[PathBuf],
    tool_version: Option<&str>,
    module: &str,
    module_span: &Option<Span>,
) -> Vec<Diagnostic> {
    let request = LintRequest {
        op: "lint",
        paths,
        tool_version,
    };
    run_exchange(
        &mut Command::new(exe),
        name,
        &request,
        module,
        module_span,
        LINT_TIMEOUT,
    )
}

fn run_exchange(
    command: &mut Command,
    name: &str,
    request: &impl serde::Serialize,
    module: &str,
    module_span: &Option<Span>,
    timeout: Duration,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut responded = false;
    let mut too_many_diagnostics = false;
    let mut input = gripsack_process::InputBuffer::new(Limits::default().input_bytes);
    let encoded = serde_json::to_writer(&mut input, request)
        .map_err(|error| error.to_string())
        .and_then(|()| {
            std::io::Write::write_all(&mut input, b"\n").map_err(|error| error.to_string())
        });
    if let Err(error) = encoded {
        return vec![host_diagnostic(
            name,
            "E02",
            Severity::Error,
            error,
            module,
            module_span,
        )];
    }
    let outcome = gripsack_process::run(
        command,
        input.as_bytes(),
        Limits {
            timeout,
            ..Limits::default()
        },
        |line| {
            let line = String::from_utf8_lossy(line);
            let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
                return Control::Continue;
            };
            match msg.get("type").and_then(|t| t.as_str()) {
                Some("diagnostic") => {
                    if let Some(raw) = msg.get("diagnostic") {
                        if diagnostics.len() == 1024 {
                            too_many_diagnostics = true;
                            return Control::Response;
                        }
                        diagnostics.push(from_plugin(raw, module, module_span));
                    }
                }
                Some("response") => {
                    responded = true;
                    return Control::Response;
                }
                _ => {}
            }
            Control::Continue
        },
    );
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            return vec![host_diagnostic(
                name,
                "E01",
                Severity::Error,
                format!(
                    "cannot run {}: {error}",
                    command.get_program().to_string_lossy()
                ),
                module,
                module_span,
            )];
        }
    };
    if too_many_diagnostics {
        diagnostics.push(host_diagnostic(
            name,
            "E02",
            Severity::Error,
            "linter exceeded the 1024 diagnostic cap".into(),
            module,
            module_span,
        ));
    }
    // A linter may use a nonzero exit for valid diagnostics, but must
    // actually exit and complete the protocol. Transport failure always E02.
    if matches!(outcome.reason, StopReason::Exited)
        && (responded || too_many_diagnostics)
        && outcome.status.is_some()
    {
        return diagnostics;
    }
    let message = match &outcome.reason {
        StopReason::Deadline => format!(
            "linter {name:?} exceeded the {}s exchange deadline and was killed — the linter hung, not the config",
            timeout.as_secs()
        ),
        StopReason::LineLimit => format!(
            "linter {name:?} wrote a single line over the 1 MiB cap and was killed — the linter is broken, not the config"
        ),
        StopReason::Exited => format!(
            "linter {name:?} exited {} without a response — the linter is broken, not the config",
            outcome
                .status
                .map(|s| s.to_string())
                .unwrap_or_else(|| "?".into())
        ),
        other => format!(
            "linter {name:?} exchange failed: {other:?} — the linter is broken, not the config"
        ),
    };
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    let tail = stderr
        .lines()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let mut diagnostic =
        host_diagnostic(name, "E02", Severity::Warning, message, module, module_span);
    // Keep stderr first, ahead of the module callsite label.
    diagnostic.labels.insert(
        0,
        gripsack_ir::Label {
            span: None,
            note: if tail.is_empty() {
                "no stderr".into()
            } else {
                format!("stderr tail:\n{tail}")
            },
        },
    );
    diagnostics.push(diagnostic);
    diagnostics
}
