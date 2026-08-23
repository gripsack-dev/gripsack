//! Command implementations: eval wiring (0005 §4) and the lifecycle
//! commands against the store (apply / generations / rollback).

use crate::render::{self, Palette};
use gripsack_exec::{Ctx, Outcome};
use gripsack_ir::Ir;
use gripsack_store as store;
use owo_colors::OwoColorize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tracing::info;

/// Evaluate an env repo's frontend into IR JSON (0005 §4). The core
/// never embeds a runtime — this is a subprocess.
pub fn eval_repo(repo: &Path, host: Option<&str>, palette: Palette) -> Result<String, ExitCode> {
    let env_path = repo.join("env.toml");
    if !env_path.exists() {
        eprintln!(
            "grip: no env.toml in {} — is this an env repo?",
            repo.display()
        );
        return Err(ExitCode::FAILURE);
    }
    let env = match gripsack_config::load_env(&env_path) {
        Ok(env) => env,
        Err(diagnostics) => {
            eprintln!("{}", render::render_diagnostics(&diagnostics, palette));
            return Err(ExitCode::FAILURE);
        }
    };
    if env.env.frontend != gripsack_config::Frontend::Python {
        eprintln!("grip: typescript eval lands in 0.2 — set `frontend = \"python\"` for now");
        return Err(ExitCode::from(2));
    }
    let mut cmd = std::process::Command::new("python3");
    cmd.arg("-m").arg("gripsack").arg(repo).current_dir(repo);
    let host = host
        .map(str::to_string)
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "default".into());
    cmd.arg("--host").arg(&host);
    let out = cmd.output().map_err(|e| {
        eprintln!("grip: cannot spawn python3: {e} (see `grip doctor`)");
        ExitCode::FAILURE
    })?;
    if !out.status.success() {
        // Frontend tracebacks are the frontend's domain (0005 §4) —
        // pass them through untouched.
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        eprintln!("grip: frontend eval failed ({host})");
        return Err(ExitCode::FAILURE);
    }
    String::from_utf8(out.stdout).map_err(|_| {
        eprintln!("grip: frontend emitted non-utf8 IR — this is a frontend bug");
        ExitCode::FAILURE
    })
}

/// Parse + validate IR, rendering diagnostics on failure.
pub fn check_ir(json: &str, palette: Palette) -> Result<Ir, ExitCode> {
    gripsack_ir::check(json).map_err(|diagnostics| {
        for d in &diagnostics {
            tracing::error!(code = d.code.as_ref(), "{}", d.message);
        }
        eprintln!("{}", render::render_diagnostics(&diagnostics, palette));
        ExitCode::FAILURE
    })
}

/// grip apply: eval → validate → execute → new generation (or satisfied).
pub fn apply(repo: &Path, host: Option<&str>, modules: Vec<String>, palette: Palette) -> ExitCode {
    let json = match eval_repo(repo, host, palette) {
        Ok(j) => j,
        Err(code) => return code,
    };
    let ir = match check_ir(&json, palette) {
        Ok(ir) => ir,
        Err(code) => return code,
    };
    let ctx = Ctx {
        home: store::gripsack_home(),
        repo: repo.to_path_buf(),
        only: modules,
    };
    let started = std::time::Instant::now();
    match gripsack_exec::apply(&ir, &ctx) {
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

/// grip generations: list, marking the active one.
pub fn generations() -> ExitCode {
    let home = store::gripsack_home();
    let current = store::current_generation(&home);
    let all = store::list_generations(&home);
    if all.is_empty() {
        println!("no generations yet — run `grip apply`");
        return ExitCode::SUCCESS;
    }
    for n in all {
        let modules = store::read_manifest(&home, n)
            .map(|g| g.modules.len())
            .unwrap_or(0);
        let marker = if Some(n) == current { "→" } else { " " };
        println!("{marker} {n:>3}  ({modules} modules)");
    }
    ExitCode::SUCCESS
}

/// grip rollback: restore a generation's deployment, then flip back.
pub fn rollback(generation: Option<u64>) -> ExitCode {
    let home = store::gripsack_home();
    let current = store::current_generation(&home);
    let target = match (generation, current) {
        (Some(n), _) => n,
        (None, Some(c)) if c > 1 => c - 1,
        (None, _) => {
            eprintln!("grip: nothing to roll back to");
            return ExitCode::FAILURE;
        }
    };
    let manifest = match store::read_manifest(&home, target) {
        Ok(m) => m,
        Err(_) => {
            eprintln!("grip: no generation {target}");
            return ExitCode::FAILURE;
        }
    };
    // Remove destinations the target generation doesn't know about.
    if let Some(c) = current
        && let Ok(current_manifest) = store::read_manifest(&home, c)
    {
        for (name, state) in &current_manifest.modules {
            let target_entries = manifest.modules.get(name);
            for entry in &state.entries {
                let still = target_entries
                    .map(|s| s.entries.iter().any(|e| e.to == entry.to))
                    .unwrap_or(false);
                if !still {
                    let _ = std::fs::remove_file(expand_home(&entry.to));
                }
            }
        }
    }
    for state in manifest.modules.values() {
        for entry in &state.entries {
            let source = state.store_path.join(&entry.from);
            let dest = expand_home(&entry.to);
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let result = match entry.mode {
                gripsack_ir::Ownership::Owned => store::symlink_replace(&dest, &source),
                _ => std::fs::read(&source).and_then(|bytes| store::atomic_write(&dest, &bytes)),
            };
            if let Err(e) = result {
                eprintln!("grip: rollback failed for {}: {e}", dest.display());
                return ExitCode::FAILURE;
            }
        }
    }
    if let Err(e) = store::flip(&home, target) {
        eprintln!("grip: cannot flip to generation {target}: {e}");
        return ExitCode::FAILURE;
    }
    info!(generation = target, "rolled back");
    println!("{} generation {}", "rolled back to".green().bold(), target);
    ExitCode::SUCCESS
}

fn expand_home(to: &str) -> PathBuf {
    if let Some(rest) = to.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(to)
}
