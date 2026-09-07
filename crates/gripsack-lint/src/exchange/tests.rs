use super::*;
use std::time::Instant;

fn exchange(body: &str, request: serde_json::Value) -> Vec<Diagnostic> {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", body]);
    run_exchange(
        &mut command,
        "test",
        &request,
        "mod",
        &None,
        Duration::from_secs(3),
    )
}

fn has_failure(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(|d| d.code.ends_with("/E02"))
}

#[test]
fn severity_coercion_and_crash_codes() {
    for severity in [
        "error", "ERROR", "Error", "warning", "WARNING", "info", "note", "hint", "fatal", "",
    ] {
        let d = from_plugin(
            &serde_json::json!({"code":"X1","severity":severity}),
            "mod",
            &None,
        );
        assert_eq!(
            d.severity,
            if severity.eq_ignore_ascii_case("error") {
                Severity::Error
            } else {
                Severity::Warning
            }
        );
    }
    assert_eq!(
        from_plugin(&serde_json::json!({}), "mod", &None).severity,
        Severity::Warning
    );
    for code in ["E99", "griplint-x/E99", "griplint-x/E02"] {
        let d = from_plugin(
            &serde_json::json!({"code":code,"severity":"error"}),
            "mod",
            &None,
        );
        assert_eq!(d.severity, Severity::Warning);
    }
}

#[test]
fn valid_diagnostics_survive_nonzero_exit() {
    let diagnostics = exchange(
        "read line; echo noise; echo '{\"type\":\"diagnostic\",\"diagnostic\":{\"code\":\"X1\",\"severity\":\"error\",\"message\":\"boom\"}}'; printf '{\"type\":\"response\"}'; exit 1",
        serde_json::json!({}),
    );
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code.as_ref(), "X1");
    assert_eq!(diagnostics[0].severity, Severity::Error);
    assert_eq!(diagnostics[0].message, "boom");
}

#[test]
fn silence_and_response_then_linger_are_deadline_failures() {
    for body in [
        "read line; exec sleep 60",
        "read line; echo '{\"type\":\"response\"}'; exec sleep 60",
    ] {
        let start = Instant::now();
        let diagnostics = exchange(body, serde_json::json!({}));
        assert!(start.elapsed() < Duration::from_secs(15));
        assert!(has_failure(&diagnostics));
        assert!(
            diagnostics
                .last()
                .unwrap()
                .message
                .contains("exchange deadline")
        );
    }
}

#[test]
fn output_failure_after_response_keeps_diagnostics_and_adds_e02() {
    let diagnostics = exchange(
        "read line; echo '{\"type\":\"diagnostic\",\"diagnostic\":{\"code\":\"X1\"}}'; echo '{\"type\":\"response\"}'; head -c 2097152 /dev/zero",
        serde_json::json!({}),
    );
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].code.as_ref(), "X1");
    assert!(diagnostics[1].message.contains("1 MiB cap"));
    assert_eq!(diagnostics[1].severity, Severity::Warning);
}

#[test]
fn pressure_before_input_and_response_before_input_completion() {
    let diagnostics = exchange(
        "head -c 262144 /dev/zero >&2; echo noise; echo '{\"type\":\"response\"}'; cat >/dev/null",
        serde_json::json!({"paths":["x".repeat(512 * 1024)]}),
    );
    assert!(diagnostics.is_empty());
}

#[test]
fn missing_response_retains_only_last_three_stderr_lines() {
    let diagnostics = exchange(
        "cat >/dev/null; printf 'one\ntwo\nthree\nfour\n' >&2",
        serde_json::json!({}),
    );
    assert!(has_failure(&diagnostics));
    assert!(diagnostics[0].message.contains("without a response"));
    assert_eq!(
        diagnostics[0].labels[0].note,
        "stderr tail:\ntwo\nthree\nfour"
    );
}

#[test]
fn input_limit_is_not_a_success() {
    let diagnostics = run_exchange(
        &mut Command::new("/nonexistent/gripsack-linter"),
        "oversized",
        &serde_json::json!({"paths":["x".repeat(4 * 1024 * 1024)]}),
        "mod",
        &None,
        Duration::from_secs(3),
    );
    // Input is rejected before trying the nonexistent executable. Attempting
    // to spawn instead would yield E01, not this fatal request-budget error.
    assert!(has_failure(&diagnostics));
    assert_eq!(diagnostics[0].severity, Severity::Error);
}

#[test]
fn spawn_failure_is_e01() {
    let diagnostics = run_linter(
        Path::new("/nonexistent/gripsack-linter"),
        "missing",
        &[],
        None,
        "mod",
        &None,
    );
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].code.ends_with("/E01"));
    assert_eq!(diagnostics[0].severity, Severity::Error);
}
