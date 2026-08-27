use crate::render::Palette;
use owo_colors::OwoColorize;
use std::process::ExitCode;

/// grip gc: collect store paths no generation references, and prune
/// generations beyond keep_generations (user config) — never current.
/// --dry-run previews without deleting (plan-before-apply, N6).
pub fn gc(palette: Palette, dry_run: bool) -> ExitCode {
    let home = gripsack_store::gripsack_home();
    // gc deletes store paths an in-flight apply may have published but
    // not yet flipped — it must hold the same lifecycle lock as apply
    // and rollback (finding D: same one-line fix, same family)
    let _lifecycle_lock = match gripsack_exec::acquire_lifecycle_lock(&home) {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("grip: cannot take the apply lock: {e}");
            return ExitCode::FAILURE;
        }
    };
    let keep = user_keep_generations();
    match gripsack_exec::gc(&home, keep, dry_run) {
        Ok(report) => {
            if dry_run {
                println!("{}", "gc (dry run): nothing deleted".dimmed());
            }
            if report.generations_removed.is_empty() && report.store_removed.is_empty() {
                println!("{}", "gc: nothing to collect".dimmed());
            } else {
                if !report.generations_removed.is_empty() {
                    println!(
                        "{} pruned generations {}",
                        "gc:".green().bold(),
                        report
                            .generations_removed
                            .iter()
                            .map(u64::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                for path in &report.store_removed {
                    println!("{} collected {}", "gc:".green().bold(), path.display());
                }
                if palette.enabled || !report.store_removed.is_empty() {
                    println!("{} freed", format_size(report.bytes_freed));
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", format!("error: {e}").red().bold());
            ExitCode::FAILURE
        }
    }
}

/// keep_generations: an env.toml next to your cwd wins (the same
/// default apply uses without --repo), then the user layer (the
/// documented repo-over-user precedence).
fn user_keep_generations() -> Option<u32> {
    let repo = std::env::current_dir()
        .ok()
        .map(|d| d.join("env.toml"))
        .filter(|p| p.exists());
    if let Some(path) = repo
        && let Ok(source) = std::fs::read_to_string(path)
        && let Ok(env) = gripsack_config::parse_env(&source)
        && env.settings.keep_generations.is_some()
    {
        return env.settings.keep_generations;
    }
    let path = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .map(|h| h.join(".config/gripsack/config.toml"))?;
    let source = std::fs::read_to_string(path).ok()?;
    gripsack_config::parse_user(&source)
        .ok()?
        .settings
        .keep_generations
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}
