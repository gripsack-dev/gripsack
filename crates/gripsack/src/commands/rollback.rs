use crate::render::Palette;
use gripsack_store as store;
use std::process::ExitCode;
use tracing::info;

/// grip rollback: restore a generation's deployment, then flip back —
/// through the same journaled transaction protocol as apply
/// (plan/0025 §A): a crash mid-rollback is recovered by the next
/// run's reconcile, and an ordinary failure restores the pre-rollback
/// state before returning.
pub fn rollback(generation: Option<u64>, palette: Palette) -> ExitCode {
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
    let current = match store::current_generation(&home) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("grip: cannot read the current generation: {e}");
            return ExitCode::FAILURE;
        }
    };
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
    // fail closed (0027 §3): an unreadable CURRENT manifest blocks
    // the rollback — the transition map would be built without the
    // authoritative live state
    let current_manifest = match current {
        Some(c) => match store::read_manifest(&home, c) {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("grip: current generation {c}'s manifest is unreadable: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    match gripsack_exec::rollback_generation(&home, current_manifest.as_ref(), &manifest) {
        Ok(notes) => {
            for note in notes {
                println!("  {note}");
            }
            info!(generation = target, "rolled back");
            println!("{} generation {}", palette.good("rolled back to"), target);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("grip: rollback failed: {e}");
            ExitCode::FAILURE
        }
    }
}
