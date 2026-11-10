use super::eval::{EvalOutcome, eval_repo};
use crate::live::{Detail, LiveRenderer};
use crate::render::Palette;
use gripsack_exec::{Ctx, Outcome};
use gripsack_store as store;
use std::path::Path;
use std::process::ExitCode;

/// Profile selection and destination ownership authority for one apply.
pub struct ApplyOptions {
    pub host: Option<String>,
    pub profiles: Vec<String>,
    pub take_over: bool,
    pub take_over_entries: Option<std::collections::BTreeSet<String>>,
    /// How much passing check/hook output to show (failures show all).
    pub detail: Detail,
}

impl ApplyOptions {
    fn scoped(profile: String, entries: std::collections::BTreeSet<String>) -> Self {
        ApplyOptions {
            host: None,
            profiles: vec![profile],
            take_over: false,
            take_over_entries: Some(entries),
            detail: Detail::Collapsed,
        }
    }
}

/// grip apply: eval → validate → execute → new generation (or satisfied).
pub fn apply(repo: &Path, opts: ApplyOptions, palette: Palette) -> ExitCode {
    apply_inner(repo, opts, palette)
}

/// Apply with scoped take-over (0015 §3): `grip adopt` absorbs exactly
/// the destinations it generated — unrelated drift is never clobbered.
pub fn apply_scoped(
    outcome: EvalOutcome,
    profile: String,
    entries: std::collections::BTreeSet<String>,
    palette: Palette,
) -> ExitCode {
    let opts = ApplyOptions::scoped(profile, entries);
    let mut sink =
        crate::render::DiagnosticSink::terminal(palette, outcome.sources.repository_identity());
    sink.bind_captured_source(std::sync::Arc::clone(&outcome.sources));
    apply_evaluated(outcome, opts, palette, &mut sink)
}

fn apply_inner(repo: &Path, mut opts: ApplyOptions, palette: Palette) -> ExitCode {
    let host = opts.host.take();
    let mut sink = crate::render::DiagnosticSink::terminal(palette, repo);
    LiveRenderer::announce_evaluation();
    let outcome = match eval_repo(repo, host, &mut sink) {
        Ok(o) => o,
        Err(code) => return code,
    };
    apply_evaluated(outcome, opts, palette, &mut sink)
}

fn apply_evaluated(
    outcome: EvalOutcome,
    opts: ApplyOptions,
    palette: Palette,
    sink: &mut crate::render::DiagnosticSink,
) -> ExitCode {
    let ir = match crate::commands::validated_ir(&outcome, sink) {
        Ok(ir) => ir,
        Err(code) => return code,
    };
    if let Err(code) = crate::commands::reject_workspace_execution(
        &ir,
        gripsack_ir::workspace::WorkspaceOperation::Apply,
        sink,
    ) {
        return code;
    }
    let live = std::sync::Arc::new(LiveRenderer::new(palette, opts.detail, &ir));
    let ctx = Ctx {
        home: store::gripsack_home(),
        home_dir: Default::default(),
        repository: gripsack_exec::Repository::evaluated_with_inputs(
            std::sync::Arc::clone(&outcome.sources),
            outcome.receipt,
            outcome.host_inputs.facts,
            outcome.host_inputs.probes,
        ),
        only: opts.profiles,
        host: outcome.host.clone(),
        take_over: opts.take_over,
        take_over_entries: opts.take_over_entries,
        fetch: std::sync::Arc::clone(&outcome.fetch),
        on_progress: Some({
            let live = std::sync::Arc::clone(&live);
            Box::new(move |event| live.render(event))
        }),
    };
    let started = std::time::Instant::now();
    let result = gripsack_exec::apply(&ir, &ctx);
    gripsack_fetch::throttle::save_global();
    live.finish();
    match result {
        Ok(result) => {
            // every step and verification already streamed as it happened
            let elapsed = palette.dim(&format!("{:.1}s", started.elapsed().as_secs_f32()));
            match result.outcome {
                Outcome::Satisfied { generation } => println!(
                    "{} (generation {}, {elapsed})",
                    palette.good("already satisfied"),
                    generation.map(|n| n.to_string()).unwrap_or("—".into()),
                ),
                Outcome::Applied { generation } => println!(
                    "{} generation {generation} active ({elapsed})",
                    palette.good("applied —"),
                ),
            }
            ExitCode::SUCCESS
        }
        Err(gripsack_exec::ExecError::Fetch(gripsack_fetch::FetchError::Diagnostics(
            diagnostics,
        ))) => {
            // plugin diagnostics render through the one renderer (0009 §2)
            sink.report(&diagnostics);
            ExitCode::FAILURE
        }
        Err(gripsack_exec::ExecError::Gate(d)) => {
            // pre-mutation validity gates carry their own spans —
            // render them as-is, same as sema (0030 §P0-1)
            sink.report(&[d]);
            ExitCode::FAILURE
        }
        Err(e) => {
            // Execution failures carry the named output's declaration span.
            let (code, output) = match &e {
                gripsack_exec::ExecError::Step { output, .. } => {
                    (gripsack_ir::codes::EXEC_STEP, Some(output.as_str()))
                }
                _ => (gripsack_ir::codes::EXEC_FETCH, None),
            };
            let span = output.and_then(|name| {
                ir.workspace_outputs()
                    .find(|output| output.name == name)
                    .map(|output| output.span.clone())
            });
            let mut d = gripsack_ir::Diagnostic::error(code, e.to_string());
            if let Some(span) = span {
                d = d.with_label(Some(span), "raised here");
            }
            sink.report(&[d]);
            ExitCode::FAILURE
        }
    }
}
