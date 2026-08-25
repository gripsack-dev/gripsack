use owo_colors::OwoColorize;
use std::process::ExitCode;

/// grip why-owns <path>: which module owns a deployed path, per the
/// current generation's manifest.
pub fn why_owns(path: &str) -> ExitCode {
    let home = gripsack_store::gripsack_home();
    match gripsack_exec::why_owns(&home, path) {
        Ok(Some((module, entry))) => {
            println!(
                "{} {} ({} → {}, {})",
                module.green().bold(),
                path,
                entry.from,
                entry.to,
                format!("{:?}", entry.mode).dimmed()
            );
            ExitCode::SUCCESS
        }
        Ok(None) => {
            eprintln!("grip: no module owns {path} in the current generation");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("grip: {e}");
            ExitCode::FAILURE
        }
    }
}
