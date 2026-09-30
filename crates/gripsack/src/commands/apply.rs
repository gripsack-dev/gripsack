use super::eval::{EvalOutcome, eval_repo};
use crate::render::Palette;
use gripsack_exec::{Ctx, Outcome};
use gripsack_store as store;
use owo_colors::OwoColorize;
use std::path::Path;
use std::process::ExitCode;

/// What an apply run varies by — named, not positional (the CLI flag
/// surface, the adopt scoping, and the concurrency hint travel
/// together into the executor).
pub struct ApplyOptions {
    pub host: Option<String>,
    pub modules: Vec<String>,
    pub take_over: bool,
    pub take_over_entries: Option<std::collections::BTreeSet<String>>,
    pub jobs: Option<usize>,
}

impl ApplyOptions {
    fn scoped(entries: std::collections::BTreeSet<String>, jobs: Option<usize>) -> Self {
        ApplyOptions {
            host: None,
            modules: vec![],
            take_over: false,
            take_over_entries: Some(entries),
            jobs,
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
    entries: std::collections::BTreeSet<String>,
    jobs: Option<usize>,
    palette: Palette,
) -> ExitCode {
    let opts = ApplyOptions::scoped(entries, jobs);
    if let Err(code) = admit_jobs(&opts) {
        return code;
    }
    let mut sink =
        crate::render::DiagnosticSink::terminal(palette, outcome.sources.repository_identity());
    sink.bind_captured_source(std::sync::Arc::clone(&outcome.sources));
    apply_evaluated(outcome, opts, palette, &mut sink)
}

fn admit_jobs(opts: &ApplyOptions) -> Result<(), ExitCode> {
    if opts.jobs == Some(0) {
        eprintln!("grip: --jobs 0 would run zero modules — pass a positive count");
        return Err(ExitCode::from(2));
    }
    if opts.jobs.is_none() && std::env::var("GRIPSACK_JOBS").ok().as_deref() == Some("0") {
        eprintln!("grip: GRIPSACK_JOBS=0 would run zero modules — unset or fix it");
        return Err(ExitCode::from(2));
    }
    Ok(())
}

fn apply_inner(repo: &Path, mut opts: ApplyOptions, palette: Palette) -> ExitCode {
    if let Err(code) = admit_jobs(&opts) {
        return code;
    }
    let host = opts.host.take();
    let mut sink = crate::render::DiagnosticSink::terminal(palette, repo);
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
    let spinner = if palette.enabled {
        let pb = indicatif::ProgressBar::new_spinner();
        pb.set_style(
            indicatif::ProgressStyle::with_template("{spinner:.green} {msg}")
                .expect("static template")
                // the retro braille snake (rootle's loading eye candy,
                // adopted): a green coil that never stops looking busy
                .tick_chars("⣾⣽⣻⢿⡿⣟⣯⣷ "),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(80));
        Some(pb)
    } else {
        None
    };
    let ctx = Ctx {
        home: store::gripsack_home(),
        home_dir: Default::default(),
        repository: gripsack_exec::Repository::evaluated(
            std::sync::Arc::clone(&outcome.sources),
            outcome.receipt,
        ),
        only: opts.modules,
        host: outcome.host.clone(),
        take_over: opts.take_over,
        take_over_entries: opts.take_over_entries,
        fetch: std::sync::Arc::clone(&outcome.fetch),
        jobs: opts.jobs.or_else(|| {
            std::env::var("GRIPSACK_JOBS")
                .ok()
                .and_then(|v| v.parse().ok())
        }),
        on_progress: spinner.as_ref().map(|pb| {
            let pb = pb.clone();
            Box::new(move |module: &str, verb: &str| {
                pb.set_message(format!("{module} · {verb}"));
                pb.tick();
            }) as gripsack_exec::ProgressCallback
        }),
    };
    let started = std::time::Instant::now();
    let result = gripsack_exec::apply(&ir, &ctx);
    gripsack_fetch::throttle::save_global();
    if let Some(pb) = &spinner {
        pb.finish_and_clear();
    }
    match result {
        Ok(result) => {
            print_reports(&result.reports, palette);
            let elapsed = format!("{:.1}s", started.elapsed().as_secs_f32());
            match result.outcome {
                Outcome::Satisfied { generation } => println!(
                    "{} (generation {}, {})",
                    "already satisfied".green().bold(),
                    generation.map(|n| n.to_string()).unwrap_or("—".into()),
                    elapsed.dimmed()
                ),
                Outcome::Applied { generation } => println!(
                    "{} generation {} active ({})",
                    "applied —".green().bold(),
                    generation,
                    elapsed.dimmed()
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
            // exec failures get the same span-labeled treatment as
            // sema errors (0004 §3): step/verify errors name a module,
            // and the module's span is in the IR
            let (code, module) = match &e {
                gripsack_exec::ExecError::Step { module, .. }
                | gripsack_exec::ExecError::WorkerPanicked { module } => {
                    (gripsack_ir::codes::EXEC_STEP, Some(module.as_str()))
                }
                gripsack_exec::ExecError::Verify { module, .. } => {
                    (gripsack_ir::codes::EXEC_VERIFY, Some(module.as_str()))
                }
                _ => (gripsack_ir::codes::EXEC_FETCH, None),
            };
            let span = module
                .and_then(|name| ir.modules.get(name))
                .and_then(|m| m.span.clone());
            let mut d = gripsack_ir::Diagnostic::error(code, e.to_string());
            if let Some(span) = span {
                d = d.with_label(Some(span), "raised here");
            }
            sink.report(&[d]);
            ExitCode::FAILURE
        }
    }
}

/// The apply report: aligned module column, symbol, summary (cargo/uv
/// conventions — symbols sparse, paths dimmed, quiet by default).
fn print_reports(reports: &[gripsack_exec::StepReport], palette: Palette) {
    use gripsack_exec::ReportKind as K;
    let width = reports.iter().map(|r| r.module.len()).max().unwrap_or(0);
    for r in reports {
        let symbol = match (r.kind, palette.enabled) {
            (K::Warned, true) => "⚠".yellow().to_string(),
            (K::Satisfied, true) => "·".dimmed().to_string(),
            (_, true) => "✓".green().to_string(),
            (K::Warned, false) => "⚠".to_string(),
            (K::Satisfied, false) => "·".to_string(),
            (_, false) => "✓".to_string(),
        };
        let module = format!("{:>width$}", r.module);
        let module = if palette.enabled {
            module.cyan().to_string()
        } else {
            module
        };
        println!("  {module} {symbol} {}", r.summary);
    }
}
