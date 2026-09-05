//! The one-shot protocol exchange (0009 §2): one JSON request on
//! stdin, NDJSON diagnostics and one response on stdout. stderr drains
//! concurrently (a chatty plugin fills the pipe and deadlocks
//! otherwise); the request is written from its own thread (a request
//! past the ~64KB pipe buffer would block the write half of the
//! exchange before a byte of stdout is read); stdout is read on a
//! dedicated thread, one capped line at a time, so the deadline can
//! fire during a blocking read and a linter that never newlines can't
//! grow the buffer without bound. The child is killed and reaped on
//! expiry, never waited on forever. Death is never silent (E02), spawn
//! failure is E01.

use gripsack_ir::{Diagnostic, Severity, Span};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// Crash-class codes by construction (review finding E): the host
/// classifies by code, never by the plugin's self-reported severity.
const CRASH_CODES: [&str; 2] = ["E99", "E02"];

const LINT_TIMEOUT: Duration = Duration::from_secs(120);

const MIB: usize = 1024 * 1024;

/// Cap on one stdout line — a linter that never newlines would grow
/// the read buffer without bound; the deadline bounds time, not
/// memory.
const MAX_LINE: usize = MIB;

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
        // only an explicit "error" fails the run — every other
        // severity ("warning", "info", "note", a typo) degrades to
        // warning, never escalates into a failure
        match raw.get("severity").and_then(|s| s.as_str()) {
            Some(s) if s.eq_ignore_ascii_case("error") => Severity::Error,
            _ => Severity::Warning,
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

/// One stdout event off the reader thread: a line (newline stripped),
/// the end of the stream, or a line that blew the cap.
enum Read {
    Line(String),
    Eof,
    TooLong,
}

/// Read one line of at most MAX_LINE content bytes into `buf`.
/// `read_line`/`read_until` grow the buffer without bound — the cap
/// must be checked as bytes arrive, not after the line lands.
fn read_capped_line<R: BufRead>(reader: &mut R, buf: &mut Vec<u8>) -> std::io::Result<Read> {
    buf.clear();
    loop {
        let available = match reader.fill_buf() {
            Ok(a) => a,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            return Ok(if buf.is_empty() {
                Read::Eof
            } else {
                Read::Line(stripped(buf))
            });
        }
        match available.iter().position(|&b| b == b'\n') {
            Some(nl) => {
                if buf.len() + nl > MAX_LINE {
                    return Ok(Read::TooLong);
                }
                buf.extend_from_slice(&available[..=nl]);
                reader.consume(nl + 1);
                return Ok(Read::Line(stripped(buf)));
            }
            None => {
                if buf.len() + available.len() > MAX_LINE {
                    return Ok(Read::TooLong);
                }
                buf.extend_from_slice(available);
                let n = available.len();
                reader.consume(n);
            }
        }
    }
}

/// Lossy-utf8 (a broken linter's byte soup then fails the json parse
/// downstream like any non-protocol line) with the trailing newline
/// and a CRLF carriage return stripped.
fn stripped(buf: &[u8]) -> String {
    String::from_utf8_lossy(buf)
        .trim_end_matches(['\n', '\r'])
        .to_owned()
}

/// Time left before `deadline`, ZERO once it has passed —
/// `recv_timeout(ZERO)` reports Timeout immediately, which is
/// exactly the deadline semantics the exchange wants.
fn remaining_until(deadline: Instant) -> Duration {
    let now = Instant::now();
    if deadline > now {
        deadline - now
    } else {
        Duration::ZERO
    }
}

/// Read child stdout on a dedicated thread: a blocking `read_line`
/// can't observe a deadline, the consumer's `recv_timeout` can. io
/// errors end the stream like EOF — the consumer learns why from the
/// child's exit status.
fn spawn_line_reader(stdout: std::process::ChildStdout) -> Receiver<Read> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        let mut buf = Vec::new();
        loop {
            match read_capped_line(&mut reader, &mut buf) {
                Ok(Read::Eof) | Err(_) => {
                    let _ = tx.send(Read::Eof);
                    break;
                }
                Ok(event) => {
                    if tx.send(event).is_err() {
                        break; // consumer gone; stop draining
                    }
                }
            }
        }
    });
    rx
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
    run_exchange(
        &mut child,
        name,
        &request,
        module,
        module_span,
        LINT_TIMEOUT,
    )
}

/// The conversation proper, split from `run_linter` so tests can
/// drive an already-spawned child with a short deadline instead of
/// LINT_TIMEOUT.
fn run_exchange(
    child: &mut std::process::Child,
    name: &str,
    request: &serde_json::Value,
    module: &str,
    module_span: &Option<Span>,
    timeout: Duration,
) -> Vec<Diagnostic> {
    // the request (a module linting many paths) can outrun the ~64KB
    // pipe buffer — writing it inline blocks until the peer drains,
    // wedging the exchange before a byte of stdout is read. write
    // from a thread; the pipe closes on drop, handing the peer EOF.
    // never joined: a grandchild holding the pipe open would outlive
    // the kill.
    if let Some(mut stdin) = child.stdin.take() {
        let req = request.to_string();
        std::thread::spawn(move || {
            let _ = writeln!(stdin, "{req}");
        });
    }
    let mut diagnostics = Vec::new();
    let mut responded = false;
    // why the exchange ended without a response (deadline, line cap)
    let mut failure: Option<String> = None;
    let deadline = Instant::now() + timeout;
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
    let rx = spawn_line_reader(child.stdout.take().expect("piped stdout"));
    loop {
        // the deadline lives HERE, not in the reader: recv_timeout
        // wakes while a blocking read never would
        let remaining = remaining_until(deadline);
        match rx.recv_timeout(remaining) {
            Ok(Read::Line(line)) => {
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
            Ok(Read::Eof) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                failure = Some(format!(
                    "linter {name:?} exceeded the {}s exchange deadline and was killed — \
                     the linter hung, not the config",
                    timeout.as_secs()
                ));
                break;
            }
            Ok(Read::TooLong) => {
                failure = Some(format!(
                    "linter {name:?} wrote a single line over the {} MiB cap and was \
                     killed — the linter is broken, not the config",
                    MAX_LINE / MIB
                ));
                break;
            }
        }
    }
    if failure.is_some() {
        let _ = child.kill();
    }
    // reap within the deadline; a linter that answered (or stalled)
    // but never exits is killed, not waited on forever
    let status = if failure.is_some() {
        child.wait().ok()
    } else {
        loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Ok(None) => {
                    let _ = child.kill();
                    break child.wait().ok();
                }
                Err(_) => break None,
            }
        }
    };
    if responded {
        return diagnostics;
    }
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
        message: failure.unwrap_or_else(|| {
            format!(
                "linter {name:?} exited {} without a response — the linter is \
                 broken, not the config",
                status.map(|s| s.to_string()).unwrap_or_else(|| "?".into())
            )
        }),
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
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    /// an executable /bin/sh script under a pid-scoped temp path (no
    /// tempfile dev-dep in this crate; std is enough). Each call
    /// gets a unique file (atomic counter): rewriting a path a
    /// previous child is still executing yields ETXTBSY ("Text file
    /// busy"), which CI hit as a flake.
    fn script(name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "gripsack-lint-exchange-{name}-{}-{unique}.sh",
            std::process::id()
        ));
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn spawn(exe: &Path) -> std::process::Child {
        // ETXTBSY (os error 26) races a just-written script's exec on
        // some filesystems (WSL drvfs, CI overlayfs): a bounded retry
        // is the honest answer for a test-only helper
        for attempt in 0..50 {
            let result = std::process::Command::new(exe)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn();
            match result {
                Ok(child) => return child,
                Err(e) if e.raw_os_error() == Some(26) && attempt < 49 => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => panic!("spawn {}: {e}", exe.display()),
            }
        }
        unreachable!()
    }

    fn exchange(exe: &Path, name: &str, timeout: Duration) -> Vec<Diagnostic> {
        let mut child = spawn(exe);
        let out = run_exchange(
            &mut child,
            name,
            &serde_json::json!({"op": "lint"}),
            "mod",
            &None,
            timeout,
        );
        let _ = std::fs::remove_file(exe);
        out
    }

    #[test]
    fn unknown_plugin_severities_coerce_to_warning_never_error() {
        let cases = [
            ("error", Severity::Error),
            ("ERROR", Severity::Error),
            ("Error", Severity::Error),
            ("warning", Severity::Warning),
            ("WARNING", Severity::Warning),
            ("info", Severity::Warning),
            ("note", Severity::Warning),
            ("hint", Severity::Warning),
            // unknown words degrade, never escalate into a failure
            ("fatal", Severity::Warning),
            ("", Severity::Warning),
        ];
        for (raw, want) in cases {
            let d = from_plugin(
                &serde_json::json!({"code": "X1", "severity": raw, "message": "m"}),
                "mod",
                &None,
            );
            assert_eq!(d.severity, want, "severity {raw:?}");
        }
        let absent = from_plugin(
            &serde_json::json!({"code": "X1", "message": "m"}),
            "mod",
            &None,
        );
        assert_eq!(absent.severity, Severity::Warning);
        // crash-class codes stay warnings even when self-reported error
        let crash = from_plugin(
            &serde_json::json!({"code": "griplint-x/E99", "severity": "error", "message": "m"}),
            "mod",
            &None,
        );
        assert_eq!(crash.severity, Severity::Warning);
    }

    #[test]
    fn silent_linter_hits_the_deadline_and_is_killed() {
        let exe = script("silent", "exec sleep 60");
        let start = Instant::now();
        let diags = exchange(&exe, "silent", Duration::from_secs(2));
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the deadline must fire long before the 60s child exits"
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].code.ends_with("/E02"), "code {}", diags[0].code);
        assert!(
            diags[0].message.contains("exchange deadline"),
            "message {}",
            diags[0].message
        );
    }

    #[test]
    fn oversized_stdout_line_is_refused_at_the_cap() {
        let exe = script(
            "chatty",
            "head -c 2097152 /dev/zero | tr '\\0' a\nexec sleep 60",
        );
        let start = Instant::now();
        let diags = exchange(&exe, "chatty", Duration::from_secs(60));
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the cap fires mid-line, not at the deadline"
        );
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0].message.contains("1 MiB cap"),
            "message {}",
            diags[0].message
        );
    }

    #[test]
    fn well_behaved_linter_round_trips() {
        let body = r#"read line
printf '{"type":"diagnostic","diagnostic":{"code":"X1","severity":"error","message":"boom"}}\n'
printf '{"type":"response"}\n'"#;
        let exe = script("good", body);
        let diags = exchange(&exe, "good", Duration::from_secs(60));
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code.as_ref(), "X1");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].message, "boom");
    }

    #[test]
    fn capped_reader_strips_frames_and_bounds() {
        let mut buf = Vec::new();
        let mut r = std::io::BufReader::new(&b"one\r\ntwo\n"[..]);
        assert!(matches!(
            read_capped_line(&mut r, &mut buf).unwrap(),
            Read::Line(l) if l == "one"
        ));
        assert!(matches!(
            read_capped_line(&mut r, &mut buf).unwrap(),
            Read::Line(l) if l == "two"
        ));
        assert!(matches!(
            read_capped_line(&mut r, &mut buf).unwrap(),
            Read::Eof
        ));
        // an unterminated final line is still a line
        let mut r = std::io::BufReader::new(&b"tail"[..]);
        assert!(matches!(
            read_capped_line(&mut r, &mut buf).unwrap(),
            Read::Line(l) if l == "tail"
        ));
        // past the cap is refused before the newline ever arrives
        let oversized = vec![b'a'; MAX_LINE + 1];
        let mut r = std::io::BufReader::new(&oversized[..]);
        assert!(matches!(
            read_capped_line(&mut r, &mut buf).unwrap(),
            Read::TooLong
        ));
        // exactly at the cap still reads
        let capped = vec![b'a'; MAX_LINE];
        let mut r = std::io::BufReader::new(&capped[..]);
        assert!(matches!(
            read_capped_line(&mut r, &mut buf).unwrap(),
            Read::Line(l) if l.len() == MAX_LINE
        ));
    }
}
