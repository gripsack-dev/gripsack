//! `grip` — the gripsack CLI.
//!
//! ```text
//! grip doctor          frontend environment check (python · node · packages)
//! grip plan --ir FILE  validate IR, print diagnostics + execution waves
//! grip apply|rollback|generations|gc|why-owns
//!                      the executor (plan/0001–0008) — in progress
//! ```
//!
//! Colors and source snippets live in [`render`]; they follow the
//! terminal — piped output is plain.

mod commands;
mod render;

use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;
use render::Palette;

use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "grip",
    version,
    about = "gripsack — your whole environment in one bag"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Fetch, build, and deploy modules — one new generation per run
    Apply {
        /// Host entrypoint (default: this machine's hostname)
        #[arg(long)]
        host: Option<String>,
        /// Restrict to these modules (default: the whole graph)
        modules: Vec<String>,
    },
    /// Show what an apply would change, without changing anything.
    /// For now: validate IR and show the execution waves.
    Plan {
        #[arg(long)]
        host: Option<String>,
        modules: Vec<String>,
        /// Read IR JSON directly (frontend debugging)
        #[arg(long)]
        ir: Option<PathBuf>,
    },
    /// Flip `current` back to a previous generation
    Rollback {
        /// Generation number (default: the previous one)
        generation: Option<u64>,
    },
    /// List generations and their status
    Generations,
    /// Collect store paths no generation references
    Gc,
    /// Show which module owns a deployed path
    WhyOwns { path: String },
    /// Check the frontend environment (python + node + gripsack package)
    Doctor,
}

fn main() -> ExitCode {
    let palette = Palette::detect();
    let cli = Cli::parse();
    let command_name = format!("{:?}", cli.command)
        .split([' ', '('])
        .next()
        .unwrap_or("unknown")
        .to_string();
    let home = gripsack_store::gripsack_home();
    let run = gripsack_trace::init(&home).ok();
    let _run_span = run.map(|r| gripsack_trace::run_span!(r, command_name).entered());
    match cli.command {
        Command::Doctor => doctor(palette),
        Command::Apply { host, modules } => {
            let repo = std::env::current_dir().unwrap_or_default();
            commands::apply(&repo, host.as_deref(), modules, palette)
        }
        Command::Generations => commands::generations(),
        Command::Rollback { generation } => commands::rollback(generation),
        Command::Plan {
            ir: Some(path),
            modules,
            ..
        } => match modules.first() {
            Some(name) => plan_module(&path, name, palette),
            None => plan_ir(&path, palette),
        },
        Command::Plan {
            ir: None,
            host,
            modules,
        } => {
            let repo = std::env::current_dir().unwrap_or_default();
            match commands::eval_repo(&repo, host.as_deref(), palette)
                .and_then(|json| commands::check_ir(&json, palette))
            {
                Ok(ir) => {
                    let waves = gripsack_exec::waves(&ir).unwrap_or_default();
                    match modules.first() {
                        Some(name) => {
                            println!("{}", render::render_module(&ir, name, &waves, palette))
                        }
                        None => {
                            println!("{} {} modules", "plan:".green().bold(), ir.modules.len());
                            for (i, wave) in waves.iter().enumerate() {
                                println!(
                                    "  {} {}",
                                    format!("wave {i}").blue().bold(),
                                    wave.join(", ")
                                );
                            }
                        }
                    }
                    ExitCode::SUCCESS
                }
                Err(code) => code,
            }
        }
        other => {
            eprintln!("grip: `{other:?}` is not implemented yet — see plan/0001-architecture.md");
            eprintln!("      (try `grip plan --ir <file>` or `grip doctor`)");
            ExitCode::from(2)
        }
    }
}

/// Validate an IR file and show the execution waves (0004 §4, 0007 §5).
#[tracing::instrument(name = "plan", skip(palette), fields(file = %path.display()))]
fn plan_ir(path: &PathBuf, palette: Palette) -> ExitCode {
    let json = match std::fs::read_to_string(path) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("grip: cannot read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let ir = match gripsack_ir::check(&json) {
        Ok(ir) => ir,
        Err(diagnostics) => {
            for d in &diagnostics {
                tracing::error!(code = d.code.as_ref(), "{}", d.message);
            }
            eprintln!("{}", render::render_diagnostics(&diagnostics, palette));
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(modules = ir.modules.len(), "ir parsed and validated");
    let host = &ir.host;
    println!(
        "{} {} modules · host {}/{} · tags: {}",
        "plan:".green().bold(),
        ir.modules.len(),
        host.os,
        host.arch,
        if host.tags.is_empty() {
            "(none)".to_string()
        } else {
            host.tags.join(", ")
        }
    );
    match gripsack_exec::waves(&ir) {
        Ok(waves) => {
            for (i, wave) in waves.iter().enumerate() {
                println!(
                    "  {} {}",
                    format!("wave {i}").blue().bold(),
                    wave.join(", ")
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", format!("error: {e}").red().bold());
            ExitCode::FAILURE
        }
    }
}

/// Module-scoped view: `grip plan --ir FILE <module>` (0007 §5).
fn plan_module(path: &PathBuf, name: &str, palette: Palette) -> ExitCode {
    let json = match std::fs::read_to_string(path) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("grip: cannot read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let ir = match gripsack_ir::check(&json) {
        Ok(ir) => ir,
        Err(diagnostics) => {
            eprintln!("{}", render::render_diagnostics(&diagnostics, palette));
            return ExitCode::FAILURE;
        }
    };
    if !ir.modules.contains_key(name) {
        eprintln!(
            "grip: no module {name:?} in the graph (have: {})",
            ir.modules.keys().cloned().collect::<Vec<_>>().join(", ")
        );
        return ExitCode::FAILURE;
    }
    let waves = gripsack_exec::waves(&ir).unwrap_or_default();
    println!("{}", render::render_module(&ir, name, &waves, palette));
    ExitCode::SUCCESS
}

/// The frontend contract (plan/0003 §8): a Python with the `gripsack`
/// package importable; node for TypeScript repos (0005 §1).
fn doctor(palette: Palette) -> ExitCode {
    let mut ok = true;
    let colored = palette.enabled;
    let mark = |good: bool| {
        if !colored {
            return if good { "ok  " } else { "MISS" }.to_string();
        }
        if good {
            "ok  ".green().bold().to_string()
        } else {
            "MISS".red().bold().to_string()
        }
    };

    match std::process::Command::new("python3")
        .arg("--version")
        .output()
    {
        Ok(out) if out.status.success() => {
            let v = String::from_utf8_lossy(&out.stdout);
            let v = v.trim();
            let v = if v.is_empty() {
                String::from_utf8_lossy(&out.stderr).trim().to_string()
            } else {
                v.to_string()
            };
            println!("{}  python: {v}", mark(true));
        }
        _ => {
            println!("{}  python: `python3` not found on PATH", mark(false));
            ok = false;
        }
    }

    let check = std::process::Command::new("python3")
        .args(["-c", "import gripsack; print(gripsack.__version__)"])
        .output();
    match check {
        Ok(out) if out.status.success() => {
            println!(
                "{}  frontend: gripsack python {}",
                mark(true),
                String::from_utf8_lossy(&out.stdout).trim()
            );
        }
        _ => {
            println!(
                "{}  frontend: `import gripsack` failed — pip install gripsack",
                mark(false)
            );
            ok = false;
        }
    }

    // TypeScript frontend (plan/0005 §1): optional.
    match std::process::Command::new("node").arg("--version").output() {
        Ok(out) if out.status.success() => {
            println!(
                "{}  node: {} (typescript frontend)",
                mark(true),
                String::from_utf8_lossy(&out.stdout).trim()
            );
        }
        _ => println!("info  node: not found — only needed for `frontend = \"typescript\"` repos"),
    }

    println!("      home: {}", gripsack_store::gripsack_home().display());

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
