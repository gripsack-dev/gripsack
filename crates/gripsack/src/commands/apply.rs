use crate::commands::{check_ir, eval_repo, trust_gate};
use crate::render::Palette;
use gripsack_exec::{Ctx, Outcome};
use gripsack_store as store;
use owo_colors::OwoColorize;
use std::path::Path;
use std::process::ExitCode;

/// grip apply: eval → validate → execute → new generation (or satisfied).
pub fn apply(
    repo: &Path,
    host: Option<&str>,
    modules: Vec<String>,
    take_over: bool,
    jobs: Option<usize>,
    palette: Palette,
) -> ExitCode {
    apply_inner(repo, host, modules, take_over, None, jobs, palette)
}

/// Apply with scoped take-over (0015 §3): `grip adopt` absorbs exactly
/// the destinations it generated — unrelated drift is never clobbered.
pub fn apply_scoped(
    repo: &Path,
    host: Option<&str>,
    modules: Vec<String>,
    take_over_entries: Option<std::collections::BTreeSet<String>>,
    jobs: Option<usize>,
    palette: Palette,
) -> ExitCode {
    apply_inner(repo, host, modules, false, take_over_entries, jobs, palette)
}

#[allow(clippy::too_many_arguments)]
fn apply_inner(
    repo: &Path,
    host: Option<&str>,
    modules: Vec<String>,
    take_over: bool,
    take_over_entries: Option<std::collections::BTreeSet<String>>,
    jobs: Option<usize>,
    palette: Palette,
) -> ExitCode {
    if jobs == Some(0) {
        eprintln!("grip: --jobs 0 would run zero modules — pass a positive count");
        return ExitCode::from(2);
    }
    if std::env::var("GRIPSACK_JOBS").ok().as_deref() == Some("0") {
        eprintln!("grip: GRIPSACK_JOBS=0 would run zero modules — unset or fix it");
        return ExitCode::from(2);
    }
    if let Some(code) = trust_gate(repo) {
        return code;
    }
    let outcome = match eval_repo(repo, host, palette) {
        Ok(o) => o,
        Err(code) => return code,
    };
    let ir = match check_ir(&outcome.ir_json, palette)
        .and_then(|ir| crate::commands::validate_sources(&ir, repo, palette).map(|_| ir))
        .and_then(|ir| crate::commands::run_lints(&ir, &outcome, repo, host, palette).map(|_| ir))
    {
        Ok(ir) => ir,
        Err(code) => return code,
    };
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
        repo: repo.to_path_buf(),
        only: modules,
        host: host
            .map(str::to_string)
            .or_else(|| outcome.env.env.default_host.clone())
            .unwrap_or_else(crate::commands::hostname),
        take_over,
        take_over_entries,
        jobs: jobs.or_else(|| {
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
            eprintln!(
                "{}",
                crate::render::render_diagnostics(&diagnostics, palette)
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            // exec failures get the same span-labeled treatment as
            // sema errors (0004 §3): step/verify errors name a module,
            // and the module's span is in the IR
            let (code, module) = match &e {
                gripsack_exec::ExecError::Step { module, .. } => {
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
            eprintln!("{}", crate::render::render_diagnostics(&[d], palette));
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
