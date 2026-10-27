use crate::commands::{eval_repo, validated_ir};
use crate::render::{CheckHostReport, CheckOutputReport, CheckReport, DiagnosticSink};
use gripsack_ir::{Ir, Span};
use std::path::Path;
use std::process::ExitCode;

/// grip check: evaluate and validate workspace outputs and sources. No
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
    if ir
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
    check_workspace(&ir, workspace_outputs(&ir, &outcome.sources), sink)
}

/// The admitted catalog as report rows, without changing versioned semantics.
fn workspace_outputs(
    ir: &Ir,
    sources: &gripsack_store::source_bundle::SourceBundle,
) -> Vec<CheckOutputReport> {
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
