use crate::commands::{eval_repo, validated_ir};
use crate::render::{CheckHostReport, CheckOutputReport, CheckReport, DiagnosticSink};
use gripsack_ir::{Ir, Span};
use std::path::Path;
use std::process::ExitCode;

/// grip check: eval + IR sema + linters, then stop (0011 §9). No
/// lockfile, store payload or destination mutation. Private evaluation receipts
/// bind the source/input snapshot; captured source bytes are command-owned.
/// `--json` emits one versioned document on stdout carrying the same
/// facts the terminal renders (0052 A1-06); operational failures
/// (trust gate, missing deno) keep their stderr text in both modes.
pub fn check(repo: &Path, host: Option<String>, mut sink: DiagnosticSink) -> ExitCode {
    let outcome = match eval_repo(repo, host, &mut sink) {
        Ok(o) => o,
        Err(code) => return sink.finish_failure(code),
    };
    let repository = gripsack_exec::Repository::evaluated(
        std::sync::Arc::clone(&outcome.sources),
        outcome.receipt,
    );
    let ir = match validated_ir(&outcome, &mut sink) {
        Ok(ir) => ir,
        Err(code) => return sink.finish_failure(code),
    };
    if ir.has_workspace()
        && ir
            .workspace_execution_error(gripsack_ir::workspace::WorkspaceOperation::Plan)
            .is_none()
        && let Err(error) = gripsack_exec::inspect_known_layouts(
            &ir,
            &repository,
            &outcome.host,
            outcome.fetch.limits(),
        )
    {
        let diagnostic = match error {
            gripsack_exec::ExecError::Gate(diagnostic) => diagnostic,
            other => {
                gripsack_ir::Diagnostic::error(gripsack_ir::codes::EXEC_STEP, other.to_string())
            }
        };
        sink.report(&[diagnostic]);
        return sink.finish_failure(ExitCode::FAILURE);
    }
    match workspace_outputs(&ir, &outcome.sources) {
        Some(outputs) => check_workspace(&ir, outputs, sink),
        None => check_legacy(&ir, &outcome, &repository, sink),
    }
}

/// The admitted catalog as report rows, without changing versioned semantics.
fn workspace_outputs(
    ir: &Ir,
    sources: &gripsack_store::source_bundle::SourceBundle,
) -> Option<Vec<CheckOutputReport>> {
    ir.has_workspace().then(|| {
        ir.workspace_outputs()
            .map(|output| CheckOutputReport {
                name: output.name.to_owned(),
                kind: output.kind.to_owned(),
                span: Span {
                    file: sources.logical_text(&output.span.file).into_owned(),
                    line: output.span.line,
                    col: output.span.col,
                },
            })
            .collect()
    })
}

fn check_workspace(ir: &Ir, outputs: Vec<CheckOutputReport>, sink: DiagnosticSink) -> ExitCode {
    if sink.is_json() {
        let mut report = CheckReport::success(sink.into_collected());
        report.host = Some(CheckHostReport {
            os: ir.host.os.clone(),
            arch: ir.host.arch.clone(),
            tags: ir.host.tags.clone(),
        });
        report.outputs = Some(outputs);
        println!("{}", report.to_json());
        return ExitCode::SUCCESS;
    }
    let palette = sink.palette();
    let host = &ir.host;
    println!(
        "{} {} named outputs · host {}/{}",
        palette.good("check: ok"),
        outputs.len(),
        host.os,
        host.arch
    );
    for output in &outputs {
        println!("  {} ({})", output.name, output.kind);
    }
    ExitCode::SUCCESS
}

/// Legacy module envs: physical destination uniqueness (0030 §P0-1)
/// and known-layout inspection are read-only — two spellings of one
/// directory entry are a check-time diagnostic like any other.
fn check_legacy(
    ir: &Ir,
    outcome: &crate::commands::eval::EvalOutcome,
    repository: &gripsack_exec::Repository,
    mut sink: DiagnosticSink,
) -> ExitCode {
    let unique = gripsack_exec::expand::expand_all(&ir.modules)
        .and_then(|plans| gripsack_exec::expand::check_physical_uniqueness(&ir.modules, &plans));
    match unique {
        Ok(()) => {}
        Err(gripsack_exec::ctx::ExecError::Gate(d)) => {
            sink.report(&[d]);
            return sink.finish_failure(ExitCode::FAILURE);
        }
        Err(e) => {
            eprintln!("grip: {e}");
            return sink.finish_failure(ExitCode::FAILURE);
        }
    }
    let layouts = match gripsack_exec::inspect_known_layouts(
        ir,
        repository,
        &outcome.host,
        outcome.fetch.limits(),
    ) {
        Ok(layouts) => layouts,
        Err(error) => {
            eprintln!("grip: {error}");
            return sink.finish_failure(ExitCode::FAILURE);
        }
    };
    if sink.is_json() {
        let mut report = CheckReport::success(sink.into_collected());
        report.host = Some(CheckHostReport {
            os: ir.host.os.clone(),
            arch: ir.host.arch.clone(),
            tags: ir.host.tags.clone(),
        });
        report.modules = Some(ir.modules.keys().cloned().collect());
        let notes: std::collections::BTreeMap<String, String> = layouts
            .iter()
            .filter_map(|(module, evidence)| {
                evidence.summary().map(|s| (module.clone(), s.to_string()))
            })
            .collect();
        if !notes.is_empty() {
            report.layouts = Some(notes);
        }
        println!("{}", report.to_json());
        return ExitCode::SUCCESS;
    }
    for (module, evidence) in &layouts {
        if let Some(summary) = evidence.summary() {
            println!("  {module}: {summary}");
        }
    }
    let palette = sink.palette();
    let host = &ir.host;
    println!(
        "{} {} modules · host {}/{} · tags: {}",
        palette.good("check: ok"),
        ir.modules.len(),
        host.os,
        host.arch,
        if host.tags.is_empty() {
            "(none)".to_string()
        } else {
            host.tags.join(", ")
        }
    );
    ExitCode::SUCCESS
}
