use crate::commands::{check_ir, eval_repo};
use crate::render::Palette;
use gripsack_exec::Ctx;
use gripsack_store as store;
use owo_colors::OwoColorize;
use std::path::Path;
use std::process::ExitCode;

/// grip update: re-resolve, rewrite the lockfile, report what moved.
/// Never deploys — `grip apply` does (0008 §5).
pub fn update(repo: &Path, host: Option<&str>, modules: Vec<String>, palette: Palette) -> ExitCode {
    let json = match eval_repo(repo, host, palette) {
        Ok(j) => j,
        Err(code) => return code,
    };
    let ir = match check_ir(&json, palette) {
        Ok(ir) => ir,
        Err(code) => return code,
    };
    let host_name = host
        .map(str::to_string)
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "default".into());
    let ctx = Ctx {
        home: store::gripsack_home(),
        repo: repo.to_path_buf(),
        only: modules,
        host: host_name,
        on_progress: None,
        take_over: false,
        jobs: None,
    };
    match gripsack_exec::update(&ir, &ctx) {
        Ok(reports) => {
            if reports.is_empty() {
                println!("nothing to resolve yet — no resolvable fetches in the graph");
            }
            for r in &reports {
                match &r.status {
                    gripsack_exec::UpdateStatus::Unchanged => {
                        println!("  {} {}", r.module.cyan(), "unchanged".dimmed())
                    }
                    gripsack_exec::UpdateStatus::Bumped { .. } => {
                        println!("  {} {}", r.module.cyan(), "bumped".yellow().bold())
                    }
                    gripsack_exec::UpdateStatus::Skipped => println!(
                        "  {} {}",
                        r.module.cyan(),
                        "skipped (resolution not supported yet)".dimmed()
                    ),
                }
            }
            if reports
                .iter()
                .any(|r| matches!(r.status, gripsack_exec::UpdateStatus::Bumped { .. }))
            {
                println!("lockfile updated — run `grip apply` to deploy");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", format!("error: {e}").red().bold());
            ExitCode::FAILURE
        }
    }
}
