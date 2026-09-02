//! `grip trust` — manage the repo trust list (0013 D7): the file the
//! eval gate consults before any frontend code runs.

use crate::commands::expand_home;
use crate::render::Palette;
use clap::Subcommand;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// `grip trust` subcommands (declared here so main.rs stays a thin
/// dispatcher; `add [path]` is the non-interactive path the gate's
/// non-TTY error hints at).
#[derive(Debug, Subcommand)]
pub enum TrustCommand {
    /// List trusted repos
    List,
    /// Record a repo as trusted (no prompt — this IS the decision)
    Add {
        /// Repo path (default: current directory)
        path: Option<String>,
    },
    /// Remove a repo from the trust list
    Remove {
        /// Repo path
        path: String,
    },
}

pub fn trust(command: TrustCommand, palette: Palette) -> ExitCode {
    let home = gripsack_store::gripsack_home();
    match command {
        TrustCommand::List => list(&home, palette),
        TrustCommand::Add { path } => add(&home, &repo_path(path), palette),
        TrustCommand::Remove { path } => remove(&home, &expand_home(&path), palette),
    }
}

/// `Some(spec)` → the spec (with `~` expanded); `None` → cwd.
fn repo_path(spec: Option<String>) -> PathBuf {
    match spec {
        Some(p) => expand_home(&p),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    }
}

fn list(home: &Path, palette: Palette) -> ExitCode {
    match gripsack_store::trust::list(home) {
        Ok(repos) if repos.is_empty() => {
            println!("no trusted repos — the first eval of a repo prompts to trust it");
            ExitCode::SUCCESS
        }
        Ok(repos) => {
            for r in &repos {
                let remote = r.remote.as_deref().unwrap_or("(no remote)");
                let commit = r
                    .commit
                    .as_deref()
                    .map(|c| c.chars().take(7).collect::<String>())
                    .unwrap_or_else(|| "???????".into());
                println!(
                    "{} {} {} {}",
                    palette.cyan(&r.path),
                    palette.dim(remote),
                    palette.dim(&commit),
                    palette.dim(&format!("trusted {}", r.trusted_at))
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", palette.error(&format!("error: {e}")));
            ExitCode::FAILURE
        }
    }
}

fn add(home: &Path, path: &Path, palette: Palette) -> ExitCode {
    match gripsack_store::trust::add(home, path) {
        Ok(entry) => {
            println!("{} {}", palette.good("trusted:"), palette.cyan(&entry.path));
            println!(
                "  remote:  {}",
                palette.dim(entry.remote.as_deref().unwrap_or("(none)"))
            );
            println!(
                "  commit:  {}",
                palette.dim(
                    &entry
                        .commit
                        .as_deref()
                        .map(|c| c.chars().take(7).collect::<String>())
                        .unwrap_or_else(|| "(none)".into())
                )
            );
            println!("  at:      {}", palette.dim(&entry.trusted_at));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", palette.error(&format!("error: {e}")));
            ExitCode::FAILURE
        }
    }
}

fn remove(home: &Path, path: &Path, palette: Palette) -> ExitCode {
    match gripsack_store::trust::remove(home, path) {
        Ok(true) => {
            println!(
                "{} {}",
                palette.good("removed:"),
                gripsack_store::trust::canonical_key(path).display()
            );
            ExitCode::SUCCESS
        }
        Ok(false) => {
            eprintln!(
                "grip: {} is not in the trust list (see `grip trust list`)",
                path.display()
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("{}", palette.error(&format!("error: {e}")));
            ExitCode::FAILURE
        }
    }
}
