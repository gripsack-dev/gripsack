use crate::render::Palette;
use std::process::ExitCode;

/// grip gc: collect store paths no generation references, and prune
/// generations beyond keep_generations (user config) — never current.
/// --dry-run previews without deleting (plan-before-apply, N6).
pub fn gc(palette: Palette, dry_run: bool) -> ExitCode {
    let home = gripsack_store::gripsack_home();
    let keep = match retention_policy() {
        Ok(keep) => keep,
        Err(diagnostics) => {
            eprintln!(
                "{}",
                crate::render::render_diagnostics(&diagnostics, palette)
            );
            return ExitCode::FAILURE;
        }
    };
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
    match gripsack_exec::gc(&home, keep, dry_run) {
        Ok(report) => {
            if dry_run {
                println!("{}", palette.dim("gc (dry run): nothing deleted"));
            }
            if report.generations_removed.is_empty() && report.store_removed.is_empty() {
                println!("{}", palette.dim("gc: nothing to collect"));
            } else {
                if !report.generations_removed.is_empty() {
                    println!(
                        "{} pruned generations {}",
                        palette.good("gc:"),
                        report
                            .generations_removed
                            .iter()
                            .map(u64::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                for path in &report.store_removed {
                    println!("{} collected {}", palette.good("gc:"), path.display());
                }
                if report.bytes_freed > 0 {
                    println!("{} freed", format_size(report.bytes_freed));
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", palette.error(&format!("error: {e}")));
            ExitCode::FAILURE
        }
    }
}

/// keep_generations via the shared config layering (0005 §2): an
/// env.toml next to your cwd wins (the same default apply uses
/// without --repo), then the user layer — one merge, not a re-impl.
fn retention_policy() -> Result<Option<u32>, Vec<gripsack_ir::Diagnostic>> {
    use gripsack_ir::{Diagnostic, codes};
    let cwd = std::env::current_dir().map_err(|e| {
        vec![Diagnostic::error(
            codes::CONFIG,
            format!("cannot determine config directory: {e}"),
        )]
    })?;
    let path = cwd.join("env.toml");
    let repo = match std::fs::symlink_metadata(&path) {
        Ok(_) => gripsack_config::load_env(&path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
        Err(e) => {
            return Err(vec![Diagnostic::error(
                codes::CONFIG,
                format!("cannot inspect {}: {e}", path.display()),
            )]);
        }
    };
    let user = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .map(|home| gripsack_config::load_user(&home.join(".config/gripsack/config.toml")))
        .transpose()?;
    Ok(gripsack_config::merge(user.as_ref(), &repo)
        .settings
        .keep_generations)
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
