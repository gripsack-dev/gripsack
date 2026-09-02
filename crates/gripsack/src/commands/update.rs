use crate::commands::{check_ir, eval_repo, trust_gate};
use crate::render::Palette;
use gripsack_exec::Ctx;
use gripsack_store as store;
use owo_colors::OwoColorize;
use std::path::Path;
use std::process::ExitCode;

/// grip update: re-resolve, rewrite the lockfile, report what moved.
/// Never deploys — `grip apply` does (0008 §5).
pub fn update(repo: &Path, host: Option<&str>, modules: Vec<String>, palette: Palette) -> ExitCode {
    if let Some(code) = trust_gate(repo) {
        return code;
    }
    let outcome = match eval_repo(repo, host, palette) {
        Ok(o) => o,
        Err(code) => return code,
    };
    let ir = match check_ir(&outcome.ir_json, palette) {
        Ok(ir) => ir,
        Err(code) => return code,
    };
    let host_name = outcome.host.clone();
    let ctx = Ctx {
        home: store::gripsack_home(),
        repo: repo.to_path_buf(),
        only: modules,
        host: host_name,
        on_progress: None,
        take_over: false,
        take_over_entries: None,
        jobs: None,
    };
    gripsack_fetch::throttle::save_global();
    match gripsack_exec::update(&ir, &ctx) {
        Ok(reports) => {
            if reports.is_empty() {
                println!("nothing to resolve yet — no resolvable fetches in the graph");
            }
            for r in &reports {
                if palette.enabled {
                    match &r.status {
                        gripsack_exec::UpdateStatus::Unchanged => {
                            println!("  {} {}", r.module.cyan(), "unchanged".dimmed())
                        }
                        gripsack_exec::UpdateStatus::Bumped { .. } => {
                            println!("  {} {}", r.module.cyan(), "bumped".yellow().bold())
                        }
                        gripsack_exec::UpdateStatus::Skipped { reason } => println!(
                            "  {} {}",
                            r.module.cyan(),
                            format!("skipped ({reason})").dimmed()
                        ),
                    }
                } else {
                    match &r.status {
                        gripsack_exec::UpdateStatus::Unchanged => {
                            println!("  {} unchanged", r.module)
                        }
                        gripsack_exec::UpdateStatus::Bumped { .. } => {
                            println!("  {} bumped", r.module)
                        }
                        gripsack_exec::UpdateStatus::Skipped { reason } => {
                            println!("  {} skipped ({reason})", r.module)
                        }
                    }
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
            // colors follow the terminal (main.rs): piped output stays
            // plain
            if palette.enabled {
                eprintln!("{}", format!("error: {e}").red().bold());
            } else {
                eprintln!("error: {e}");
            }
            ExitCode::FAILURE
        }
    }
}
