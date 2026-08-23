use gripsack_store as store;
use std::process::ExitCode;

/// grip generations: list, marking the active one.
pub fn generations() -> ExitCode {
    let home = store::gripsack_home();
    let current = store::current_generation(&home);
    let all = store::list_generations(&home);
    if all.is_empty() {
        println!("no generations yet — run `grip apply`");
        return ExitCode::SUCCESS;
    }
    for n in all {
        let modules = store::read_manifest(&home, n)
            .map(|g| g.modules.len())
            .unwrap_or(0);
        let marker = if Some(n) == current { "→" } else { " " };
        println!("{marker} {n:>3}  ({modules} modules)");
    }
    ExitCode::SUCCESS
}
