use crate::render::Palette;
use std::process::ExitCode;

/// grip why-owns <path>: all module/profile ownership units in the
/// current generation, including independent blocks sharing one file.
pub fn why_owns(path: &str, palette: Palette) -> ExitCode {
    let home = gripsack_store::gripsack_home();
    match gripsack_exec::why_owns(&home, path) {
        Ok(owners) if owners.is_empty() => {
            eprintln!("grip: no module or profile owns {path} in the current generation");
            ExitCode::from(2)
        }
        Ok(owners) => {
            for owner in owners {
                for entry in owner.entries {
                    let policy = match entry.ownership.block_id() {
                        Some(block) => format!("Merge block {}", block.as_str()),
                        None => format!("{:?}", entry.ownership.policy()),
                    };
                    println!(
                        "{} {} ({} → {}, {})",
                        palette.good(&owner.name),
                        path,
                        entry.from.display(),
                        entry.to,
                        palette.dim(&policy)
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("grip: {e}");
            ExitCode::FAILURE
        }
    }
}
