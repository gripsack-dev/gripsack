use crate::commands::expand_home;
use gripsack_store as store;
use owo_colors::OwoColorize;
use std::process::ExitCode;
use tracing::info;

/// grip rollback: restore a generation's deployment, then flip back.
pub fn rollback(generation: Option<u64>) -> ExitCode {
    let home = store::gripsack_home();
    let current = store::current_generation(&home);
    let target = match (generation, current) {
        (Some(n), _) => n,
        (None, Some(c)) if c > 1 => c - 1,
        (None, _) => {
            eprintln!("grip: nothing to roll back to");
            return ExitCode::FAILURE;
        }
    };
    let manifest = match store::read_manifest(&home, target) {
        Ok(m) => m,
        Err(_) => {
            eprintln!("grip: no generation {target}");
            return ExitCode::FAILURE;
        }
    };
    // Remove destinations the target generation doesn't know about.
    if let Some(c) = current
        && let Ok(current_manifest) = store::read_manifest(&home, c)
    {
        for (name, state) in &current_manifest.modules {
            let target_entries = manifest.modules.get(name);
            for entry in &state.entries {
                let still = target_entries
                    .map(|s| s.entries.iter().any(|e| e.to == entry.to))
                    .unwrap_or(false);
                if !still {
                    let _ = std::fs::remove_file(expand_home(&entry.to));
                }
            }
        }
    }
    for state in manifest.modules.values() {
        for entry in &state.entries {
            let source = state.store_path.join(&entry.from);
            let dest = expand_home(&entry.to);
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let result = match entry.mode {
                gripsack_ir::Ownership::Owned => store::symlink_replace(&dest, &source),
                _ => std::fs::read(&source).and_then(|bytes| store::atomic_write(&dest, &bytes)),
            };
            if let Err(e) = result {
                eprintln!("grip: rollback failed for {}: {e}", dest.display());
                return ExitCode::FAILURE;
            }
        }
    }
    if let Err(e) = store::flip(&home, target) {
        eprintln!("grip: cannot flip to generation {target}: {e}");
        return ExitCode::FAILURE;
    }
    info!(generation = target, "rolled back");
    println!("{} generation {}", "rolled back to".green().bold(), target);
    ExitCode::SUCCESS
}
