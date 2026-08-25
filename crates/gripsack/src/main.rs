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
        /// Env repo path or git URL (default: current directory)
        #[arg(long)]
        repo: Option<String>,
        /// Restrict to these modules (default: the whole graph)
        modules: Vec<String>,
        /// Overwrite foreign/drifted tracked_copy destinations
        #[arg(long)]
        take_over: bool,
    },
    /// Validate the env — eval, IR sema, linters — and stop (0011 §9).
    /// Zero side effects; exit code is the CI signal.
    Check {
        /// Host entrypoint (default: this machine's hostname)
        #[arg(long)]
        host: Option<String>,
        /// Env repo path or git URL (default: current directory)
        #[arg(long)]
        repo: Option<String>,
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
        /// Env repo path or git URL (default: current directory)
        #[arg(long)]
        repo: Option<String>,
    },
    /// Flip `current` back to a previous generation
    Rollback {
        /// Generation number (default: the previous one)
        generation: Option<u64>,
    },
    /// Re-resolve and rewrite the lockfile (never deploys — apply after)
    Update {
        #[arg(long)]
        host: Option<String>,
        /// Env repo path or git URL (default: current directory)
        #[arg(long)]
        repo: Option<String>,
        modules: Vec<String>,
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
    let arg1 = std::env::args().nth(1);
    if matches!(arg1.as_deref(), Some("--version") | Some("-V")) {
        let (name, version) = ("grip", env!("CARGO_PKG_VERSION"));
        if palette.enabled {
            println!("{} {}", name.green().bold(), version.cyan());
        } else {
            println!("{name} {version}");
        }
        return ExitCode::SUCCESS;
    }
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
        Command::Doctor => commands::doctor(palette),
        Command::Apply {
            host,
            repo,
            modules,
            take_over,
        } => match commands::resolve_repo(repo.as_deref()) {
            Ok(repo) => commands::apply(&repo, host.as_deref(), modules, take_over, palette),
            Err(code) => code,
        },
        Command::Check { host, repo } => match commands::resolve_repo(repo.as_deref()) {
            Ok(repo) => commands::check(&repo, host.as_deref(), palette),
            Err(code) => code,
        },
        Command::Generations => commands::generations(),
        Command::Update {
            host,
            repo,
            modules,
        } => match commands::resolve_repo(repo.as_deref()) {
            Ok(repo) => commands::update(&repo, host.as_deref(), modules, palette),
            Err(code) => code,
        },
        Command::Rollback { generation } => commands::rollback(generation),
        Command::Plan {
            ir: Some(path),
            modules,
            ..
        } => match modules.first() {
            Some(name) => commands::plan_module(&path, name, palette),
            None => commands::plan_ir(&path, palette),
        },
        Command::Plan {
            ir: None,
            host,
            modules,
            repo,
        } => {
            let repo = match commands::resolve_repo(repo.as_deref()) {
                Ok(r) => r,
                Err(code) => return code,
            };
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
