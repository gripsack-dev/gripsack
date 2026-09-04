use gripsack_store as store;
use std::process::ExitCode;

/// grip generations: list, marking the active one.
pub fn generations() -> ExitCode {
    let home = store::gripsack_home();
    let current = match store::current_generation(&home) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("grip: cannot read the current generation: {e}");
            return ExitCode::FAILURE;
        }
    };
    let all = match store::list_generations(&home) {
        Ok(all) => all,
        Err(e) => {
            eprintln!("grip: cannot enumerate generations: {e}");
            return ExitCode::FAILURE;
        }
    };
    if all.is_empty() {
        println!("no generations yet — run `grip apply`");
        return ExitCode::SUCCESS;
    }
    for n in all {
        let modules = match store::read_manifest(&home, n) {
            Ok(g) => g.modules.len().to_string(),
            // a corrupt generation shows as corrupt (0027 §4), never
            // as "0 modules"
            Err(_) => "corrupt".to_string(),
        };
        let marker = if Some(n) == current { "→" } else { " " };
        println!("{marker} {n:>3}  ({modules} modules)");
    }
    ExitCode::SUCCESS
}
