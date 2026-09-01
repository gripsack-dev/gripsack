//! `grip store verify` — re-hash walk with --repair (0008 §3).

use crate::render::Palette;
use gripsack_exec::Ctx;
use owo_colors::OwoColorize;
use std::process::ExitCode;

pub fn store_verify(repair: bool, _palette: Palette) -> ExitCode {
    let home = gripsack_store::gripsack_home();
    // repair deletes store paths — the same lifecycle lock as apply/gc/
    // rollback, so an in-flight apply's just-published path can't vanish
    // between its manifest write and the flip
    let _lifecycle_lock = match gripsack_exec::acquire_lifecycle_lock(&home) {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("grip: cannot take the apply lock: {e}");
            return ExitCode::FAILURE;
        }
    };
    let repo = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "default".into());
    let ctx = Ctx {
        home,
        repo,
        host,
        only: Vec::new(),
        take_over: false,
        take_over_entries: None,
        jobs: None,
        on_progress: None,
    };
    match gripsack_exec::verify_store::verify_store(&ctx, repair) {
        Ok(reports) if reports.is_empty() => {
            println!("{}", "store: ok".green().bold());
            ExitCode::SUCCESS
        }
        Ok(reports) => {
            for (module, _kind, summary) in &reports {
                println!("  {} {}", module.yellow().bold(), summary);
            }
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("grip: store verify failed: {e}");
            ExitCode::FAILURE
        }
    }
}
