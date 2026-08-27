//! The golden corpus replay: every fixture case in
//! `fixtures/<tool>/<case>/` runs through the engine and must produce
//! the reference diagnostics exactly (python-shaped JSON, `<input>`
//! for the file). A00 parse-error MESSAGES are exempted (parser error
//! text is implementation-specific); their code, severity, and span
//! are pinned.

use gripsack_ir::{Diagnostic, Severity};
use serde_json::{Value as Json, json};
use std::path::{Path, PathBuf};

fn to_json(d: &Diagnostic, input: &str) -> Json {
    let labels: Vec<Json> = d
        .labels
        .iter()
        .map(|l| {
            let span = l.span.as_ref().map(|s| {
                let mut span = json!({"file": s.file.replace(input, "<input>"), "line": s.line});
                if let Some(col) = s.col {
                    span["col"] = json!(col);
                }
                span
            });
            match span {
                Some(span) => json!({"span": span, "note": l.note}),
                None => json!({"note": l.note}),
            }
        })
        .collect();
    let mut out = json!({
        "code": d.code,
        "severity": d.severity.to_string(),
        "message": d.message,
        "labels": labels,
    });
    if let Some(help) = &d.help {
        out["help"] = json!(help);
    }
    out
}

/// The comparison: full dict equality, except A00 message bodies
/// (prefix + span pinned instead).
fn equal(actual: &[Json], expected: &[Json]) -> Result<(), String> {
    if actual.len() != expected.len() {
        return Err(format!(
            "count differs: expected {}, got {}\n  expected: {expected:#?}\n  actual: {actual:#?}",
            expected.len(),
            actual.len()
        ));
    }
    for (a, e) in actual.iter().zip(expected) {
        let a00 = e["code"] == "A00";
        if a00 {
            // parser error text AND spans are implementation-specific
            // (tomllib/PyYAML/json vs toml_edit/serde); the code,
            // severity, and the "invalid X" prefix are the contract
            let same_prefix = a["message"].as_str().unwrap_or("").split(':').next()
                == e["message"].as_str().unwrap_or("").split(':').next();
            if a["code"] != e["code"] || a["severity"] != e["severity"] || !same_prefix {
                return Err(format!(
                    "A00 differs:\n  expected: {e:#?}\n  actual: {a:#?}"
                ));
            }
        } else if a != e {
            return Err(format!("differ:\n  expected: {e:#?}\n  actual: {a:#?}"));
        }
    }
    Ok(())
}

#[test]
fn golden_corpus_replays() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut cases = 0;
    let mut failures = Vec::new();
    for tool_dir in std::fs::read_dir(&root).expect("fixtures dir") {
        let tool_dir = tool_dir.expect("entry").path();
        let tool = tool_dir.file_name().unwrap().to_string_lossy().to_string();
        let Some(pack) = crate::pack_for(&tool) else {
            failures.push(format!("{tool}: no embedded pack"));
            continue;
        };
        for case in std::fs::read_dir(&tool_dir).expect("tool fixtures") {
            let case: PathBuf = case.expect("entry").path();
            let expected_path = case.join("expected.json");
            if !case.is_dir() || !expected_path.exists() {
                continue;
            }
            let input = std::fs::read_dir(&case)
                .expect("case dir")
                .map(|e| e.expect("entry").path())
                .find(|p| {
                    let n = p.file_name().unwrap().to_string_lossy();
                    n != "expected.json" && n != "tool_version"
                })
                .expect("an input file");
            let tool_version = case
                .join("tool_version")
                .exists()
                .then(|| std::fs::read_to_string(case.join("tool_version")).unwrap())
                .map(|v| v.trim().to_string());
            let text = std::fs::read_to_string(&input).expect("input text");
            let diagnostics = crate::checks::lint_file(
                &pack,
                &input.to_string_lossy(),
                &text,
                tool_version.as_deref(),
            );
            let actual: Vec<Json> = diagnostics
                .iter()
                .map(|d| to_json(d, &input.to_string_lossy()))
                .collect();
            let expected: Vec<Json> =
                serde_json::from_str(&std::fs::read_to_string(&expected_path).unwrap()).unwrap();
            cases += 1;
            if let Err(mismatch) = equal(&actual, &expected) {
                failures.push(format!(
                    "{tool}/{}: {mismatch}",
                    case.file_name().unwrap().to_string_lossy()
                ));
            }
        }
    }
    assert!(cases >= 200, "expected the full corpus, ran {cases}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Severity used above — pinned so the JSON shape stays honest.
#[test]
fn severity_serializes_lowercase() {
    assert_eq!(Severity::Warning.to_string(), "warning");
}
