use crate::commands::expand_home;
use gripsack_store as store;
use owo_colors::OwoColorize;
use std::process::ExitCode;
use tracing::info;

/// grip rollback: restore a generation's deployment, then flip back.
pub fn rollback(generation: Option<u64>) -> ExitCode {
    let home = store::gripsack_home();
    // rollback rewrites deployments and flips — same lifecycle race as
    // apply, so it holds the same lock (finding A)
    let _lifecycle_lock = match gripsack_exec::acquire_lifecycle_lock(&home) {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("grip: cannot take the apply lock: {e}");
            return ExitCode::FAILURE;
        }
    };
    let current = store::current_generation(&home);
    let target = match (generation, current) {
        (Some(n), _) => n,
        // generation 0 exists only as adopt's empty baseline (0015 §4)
        (None, Some(c)) if c >= 1 && store::read_manifest(&home, c - 1).is_ok() => c - 1,
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
    // Remove destinations the target generation doesn't know about —
    // drift-guarded, same rule as apply's prune (user edits are never
    // deleted; merge entries lose only our block)
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
                    let dest = expand_home(&entry.to);
                    // 0015 §4: an entry adopted with take-over gets its
                    // ORIGINAL file back, not a deletion
                    if !gripsack_exec::deploy::remove_or_restore_prior(&dest, entry, name, &home) {
                        tracing::warn!("kept {} — modified since deploy", entry.to);
                    }
                }
            }
        }
    }
    // restore through the ONE deploy-restore path (0001 §3.5): template
    // re-renders with the recorded vars, merge re-upserts only the
    // block — never a naive byte copy
    for (name, state) in &manifest.modules {
        for entry in &state.entries {
            let dest = expand_home(&entry.to);
            let result =
                gripsack_exec::deploy::restore_entry(&dest, entry, &state.store_path, name);
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
    // The profile tracks the generation (0001 §3.10).
    if let Err(e) = gripsack_exec::render_env_file(&home, &manifest.modules) {
        eprintln!("grip: rolled back, but env profile failed: {e}");
        return ExitCode::FAILURE;
    }
    info!(generation = target, "rolled back");
    println!("{} generation {}", "rolled back to".green().bold(), target);
    ExitCode::SUCCESS
}
