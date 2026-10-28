//! `grip store-verify` — retained payload audit and explicit repair.

use crate::render::Palette;
use std::process::ExitCode;

pub fn store_verify(repair: bool, palette: Palette) -> ExitCode {
    let home = gripsack_store::gripsack_home();
    // repair deletes store paths — the session IS the lifecycle-lock
    // contract (0045 F4), so an in-flight apply's just-published path
    // can't vanish between its manifest write and the flip.
    let session = match gripsack_exec::LifecycleSession::acquire(&home) {
        Ok(session) => session,
        Err(e) => {
            eprintln!("grip: cannot take the apply lock: {e}");
            return ExitCode::FAILURE;
        }
    };
    match gripsack_exec::verify_store::verify_store(&session, repair) {
        Ok(reports) if reports.is_empty() => {
            println!("{}", palette.good("store: ok"));
            ExitCode::SUCCESS
        }
        Ok(reports) => {
            for (output, _kind, summary) in &reports {
                println!("  {} {}", palette.warn(output), summary);
            }
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("grip: store verify failed: {e}");
            ExitCode::FAILURE
        }
    }
}
