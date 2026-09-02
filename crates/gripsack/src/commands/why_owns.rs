use crate::render::Palette;
use std::process::ExitCode;

/// grip why-owns <path>: which module owns a deployed path, per the
/// current generation's manifest.
pub fn why_owns(path: &str, palette: Palette) -> ExitCode {
    let home = gripsack_store::gripsack_home();
    match gripsack_exec::why_owns(&home, path) {
        Ok(Some((module, entry))) => {
            println!(
                "{} {} ({} → {}, {})",
                palette.good(&module),
                path,
                entry.from,
                entry.to,
                palette.dim(&format!("{:?}", entry.mode))
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
