use crate::commands::{check_ir, eval_repo};
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
    palette: Palette,
) -> ExitCode {
    let json = match eval_repo(repo, host, palette) {
        Ok(j) => j,
        Err(code) => return code,
    };
    let ir = match check_ir(&json, palette)
        .and_then(|ir| crate::commands::validate_sources(&ir, repo, palette).map(|_| ir))
    {
        Ok(ir) => ir,
        Err(code) => return code,
    };
    let spinner = if palette.enabled {
        let pb = indicatif::ProgressBar::new_spinner();
        pb.set_style(
            indicatif::ProgressStyle::with_template("{spinner:.green} {msg}")
                .expect("static template"),
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
            .or_else(|| std::env::var("HOSTNAME").ok())
            .unwrap_or_else(|| "default".into()),
        take_over,
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
            eprintln!("{}", format!("error: {e}").red().bold());
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
