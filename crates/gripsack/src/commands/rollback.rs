use crate::render::Palette;
use gripsack_host::store;
use std::process::ExitCode;
use tracing::info;

/// grip rollback: restore a generation's deployment, then flip back —
/// through the same journaled transaction protocol as apply
/// (plan/0025 §A): a crash mid-rollback is recovered by the next
/// run's reconcile, and an ordinary failure restores the pre-rollback
/// state before returning.
pub fn rollback(generation: Option<store::GenerationId>, palette: Palette) -> ExitCode {
    let home = store::gripsack_home();
    // rollback rewrites deployments and flips — the session IS the
    // lifecycle-lock contract (0045 F4), not a caller convention.
    let session = match gripsack_exec::LifecycleSession::acquire(&home) {
        Ok(session) => session,
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
    // Generation zero is adopt's historical empty baseline. A gap is not
    // permission to skip to some older retained generation.
    let target = generation.or_else(|| current.and_then(store::GenerationId::checked_previous));
    let Some(target) = target else {
        eprintln!("grip: nothing to roll back to");
        return ExitCode::FAILURE;
    };
    let manifest = match store::generations::admit_manifest(&home, target) {
        Ok(m) => m,
        Err(e) => {
            // NotFound can also mean a missing manifest *inside* a retained
            // generation. Only an absent generation explains an empty history.
            if generation.is_none()
                && e.kind() == std::io::ErrorKind::NotFound
                && matches!(
                    std::fs::symlink_metadata(store::generation_dir(&home, target)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound
                )
            {
                eprintln!("grip: nothing to roll back to");
            } else {
                eprintln!("grip: cannot admit generation {target}: {e}");
            }
            return ExitCode::FAILURE;
        }
    };
    // fail closed (0027 §3): an unreadable CURRENT manifest blocks
    // the rollback — the transition map would be built without the
    // authoritative live state
    let current_manifest = match current {
        Some(c) => match store::generations::admit_manifest(&home, c) {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("grip: current generation {c}'s manifest is unreadable: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    match gripsack_exec::rollback_generation(&session, current_manifest.as_ref(), &manifest) {
        Ok(notes) => {
            for note in notes {
                match note.severity {
                    store::journal::NoteSeverity::Warn => {
                        println!("  {} {note}", palette.warn("warning:"));
                    }
                    store::journal::NoteSeverity::Info => println!("  {note}"),
                }
            }
            info!(generation = %target, "rolled back");
            println!("{} generation {}", palette.good("rolled back to"), target);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("grip: rollback failed: {e}");
            ExitCode::FAILURE
        }
    }
}
