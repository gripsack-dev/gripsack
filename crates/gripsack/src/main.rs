//! `grip` — the gripsack CLI.
//!
//! Scaffold state: `doctor` is real (it checks the frontend environment,
//! which exists today); the remaining subcommands are the CLI surface
//! from plan/0001 §6 and report themselves unimplemented.

use clap::{Parser, Subcommand};
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
    /// Show what an apply would change, without changing anything
    Plan {
        #[arg(long)]
        host: Option<String>,
        modules: Vec<String>,
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
    /// Check the frontend environment (python + gripsack package)
    Doctor,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Doctor => doctor(),
        other => {
            eprintln!("grip: `{other:?}` is not implemented yet — see plan/0001-architecture.md");
            ExitCode::from(2)
        }
    }
}

/// The frontend contract (plan/0003 §8): a Python with the `gripsack`
/// package importable. Eval happens in that environment; the core never
/// embeds an interpreter.
fn doctor() -> ExitCode {
    let mut ok = true;

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
            println!("ok    python: {v}");
        }
        _ => {
            println!("MISS  python: `python3` not found on PATH");
            ok = false;
        }
    }

    let check = std::process::Command::new("python3")
        .args(["-c", "import gripsack; print(gripsack.__version__)"])
        .output();
    match check {
        Ok(out) if out.status.success() => {
            println!(
                "ok    frontend: gripsack python {}",
                String::from_utf8_lossy(&out.stdout).trim()
            );
        }
        _ => {
            println!("MISS  frontend: `import gripsack` failed — pip install gripsack");
            ok = false;
        }
    }

    println!("      home: {}", gripsack_store::gripsack_home().display());

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
