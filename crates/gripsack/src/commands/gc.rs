use crate::render::Palette;
use owo_colors::OwoColorize;
use std::process::ExitCode;

/// grip gc: collect store paths no generation references, and prune
/// generations beyond keep_generations (user config) — never current.
pub fn gc(palette: Palette) -> ExitCode {
    let home = gripsack_store::gripsack_home();
    let keep = user_keep_generations();
    match gripsack_exec::gc(&home, keep) {
        Ok(report) => {
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

/// keep_generations from the user layer (repo config doesn't apply to
/// a store-wide command).
fn user_keep_generations() -> Option<u32> {
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
