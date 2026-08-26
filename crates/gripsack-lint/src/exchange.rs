//! The one-shot protocol exchange (0009 §2): one JSON request on
//! stdin, NDJSON diagnostics and one response on stdout. stderr drains
//! concurrently (a chatty plugin fills the pipe and deadlocks
//! otherwise); the whole exchange has a deadline; death is never
//! silent (E02), spawn failure is E01.

use gripsack_ir::{Diagnostic, Severity, Span};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Crash-class codes by construction (review finding E): the host
/// classifies by code, never by the plugin's self-reported severity.
const CRASH_CODES: [&str; 2] = ["E99", "E02"];

const LINT_TIMEOUT: Duration = Duration::from_secs(120);

/// Coerce one plugin diagnostic (0011 §6): label-less gets the module
/// callsite; crash-class codes are warnings regardless of self-report.
fn from_plugin(raw: &serde_json::Value, module: &str, module_span: &Option<Span>) -> Diagnostic {
    let code = raw
        .get("code")
        .and_then(|c| c.as_str())
        .unwrap_or("griplint/?")
        .to_string();
    let crash_class = CRASH_CODES
        .iter()
        .any(|c| code.rsplit('/').next() == Some(c));
    let severity = if crash_class {
        Severity::Warning
    } else {
        match raw.get("severity").and_then(|s| s.as_str()) {
            Some("warning") => Severity::Warning,
            _ => Severity::Error,
        }
    };
    let mut labels = Vec::new();
    if let Some(raw_labels) = raw.get("labels").and_then(|l| l.as_array()) {
        for l in raw_labels {
            let span = l
                .get("span")
                .and_then(|s| serde_json::from_value::<Span>(s.clone()).ok());
            let note = l
                .get("note")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            labels.push(gripsack_ir::Label { span, note });
        }
    }
    if labels.is_empty()
        && let Some(span) = module_span
    {
        labels.push(gripsack_ir::Label {
            span: Some(span.clone()),
            note: format!("module {module:?} requested this lint"),
        });
    }
    Diagnostic {
        code: std::borrow::Cow::Owned(code),
        severity,
        message: raw
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("(no message)")
            .to_string(),
        labels,
        help: raw.get("help").and_then(|h| h.as_str()).map(str::to_string),
    }
}

/// One NDJSON exchange: request on stdin, diagnostics and one response
/// on stdout. Death is never silent (0009 §2.5).
pub(crate) fn run_linter(
    exe: &Path,
    name: &str,
    paths: &[PathBuf],
    tool_version: Option<&str>,
    module: &str,
    module_span: &Option<Span>,
) -> Vec<Diagnostic> {
    let request = serde_json::json!({
        "op": "lint",
        "paths": paths,
        "tool_version": tool_version,
    });
    let mut child = match std::process::Command::new(exe)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let mut d = Diagnostic {
                code: std::borrow::Cow::Owned(format!("griplint-{name}/E01")),
                severity: Severity::Error,
                message: format!("cannot run {}: {e}", exe.display()),
                labels: Vec::new(),
                help: None,
            };
            if let Some(span) = module_span {
                d = d.with_label(
                    Some(span.clone()),
                    format!("module {module:?} requested this lint"),
                );
            }
            return vec![d];
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{request}");
    }
    let mut diagnostics = Vec::new();
    let mut responded = false;
    let deadline = Instant::now() + LINT_TIMEOUT;
    // stderr must drain concurrently — a chatty linter fills the ~64KB
    // pipe buffer and blocks before it ever writes its response (the
    // fetch host learned this as review finding F1; the lint host
    // inherits the rule)
    let stderr_thread = {
        let stderr = child.stderr.take().expect("piped stderr");
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut reader = std::io::BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if buf.len() < 64 * 1024 {
                            buf.extend_from_slice(line.as_bytes());
                        }
                    }
                }
            }
            buf
        })
    };
    let mut stdout = std::io::BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut line = String::new();
    loop {
        if Instant::now() > deadline {
            let _ = child.kill();
            break;
        }
        line.clear();
        match stdout.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue; // tolerance: non-protocol lines are ignored (0009)
                };
                match msg.get("type").and_then(|t| t.as_str()) {
                    Some("diagnostic") => {
                        if let Some(raw) = msg.get("diagnostic") {
                            diagnostics.push(from_plugin(raw, module, module_span));
                        }
                    }
                    Some("response") => {
                        responded = true;
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    let status = child.wait().ok();
    if !responded {
        let stderr_buf = stderr_thread.join().unwrap_or_default();
        let stderr_tail = String::from_utf8_lossy(&stderr_buf)
            .lines()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        let mut d = Diagnostic {
            code: std::borrow::Cow::Owned(format!("griplint-{name}/E02")),
            severity: Severity::Warning,
            message: format!(
                "linter {name:?} exited {} without a response — the linter is \
                 broken, not the config",
                status.map(|s| s.to_string()).unwrap_or_else(|| "?".into())
            ),
            labels: Vec::new(),
            help: None,
        };
        d = d.with_label(
            None,
            if stderr_tail.is_empty() {
                "no stderr".to_string()
            } else {
                format!("stderr tail:\n{stderr_tail}")
            },
        );
        if let Some(span) = module_span {
            d = d.with_label(
                Some(span.clone()),
                format!("module {module:?} requested this lint"),
            );
        }
        diagnostics.push(d);
    }
    diagnostics
}
