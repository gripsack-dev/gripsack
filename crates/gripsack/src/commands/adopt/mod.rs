//! `grip adopt` — reversible adoption as a first-class flow (0015).
//!
//! Five phases, the middle three observable before anything writes to
//! disk outside the repo: inspect → ask → generate → plan → apply.
//! The apply uses scoped take-over and prior-state capture, so
//! adoption is fully reversible. The audit that shaped this module is
//! 0015 §7: ask, don't guess; say exactly what you wrote.

mod generate;
mod inspect;
mod prompt;

use crate::commands::{eval_repo, expand_home, hostname, trust_gate};
use crate::render::{self, Palette};

use gripsack_store as store;
use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub fn adopt(
    target: &str,
    name: Option<&str>,
    mode: Option<&str>,
    host: Option<&str>,
    yes: bool,
    palette: Palette,
) -> ExitCode {
    let repo = std::env::current_dir().unwrap_or_else(|_| ".".into());
    if !repo.join("env.toml").is_file() {
        eprintln!(
            "grip: {} is not an env repo (no env.toml) — run `grip init` first",
            repo.display()
        );
        return ExitCode::FAILURE;
    }
    let dest = expand_home(target);
    let home_dir = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if !dest.symlink_metadata().is_ok() {
        eprintln!("grip: nothing at {target} — adopt adopts what exists");
        return ExitCode::FAILURE;
    }
    if dest == home_dir {
        eprintln!("grip: refusing to adopt {target} — too broad");
        return ExitCode::FAILURE;
    }
    // 0015 §7 S3: absolute paths outside $HOME make non-portable repos
    if !dest.starts_with(&home_dir) {
        eprintln!(
            "grip: {target} is outside your home — adopt manages user config, not system paths"
        );
        return ExitCode::FAILURE;
    }
    let is_dir = dest.is_dir()
        && !dest
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink());
    let name = match name
        .map(str::to_string)
        .or_else(|| generate::default_name(&dest, is_dir))
    {
        Some(n) => n,
        None => {
            eprintln!("grip: cannot derive a module name from {target} — pass --name");
            return ExitCode::FAILURE;
        }
    };
    if let Some(owner) = managed_by(&dest) {
        eprintln!("grip: {target} is already managed by module \"{owner}\" — nothing to adopt");
        return ExitCode::FAILURE;
    }
    // §7 S4: the repo gets the same never-clobber rule as $HOME
    for artifact in [
        repo.join("modules").join(format!("{name}.ts")),
        repo.join("configs").join(&name),
    ] {
        if artifact.symlink_metadata().is_ok() {
            eprintln!(
                "grip: {} already exists — refusing to overwrite it (pick another --name, or remove it first)",
                artifact.display()
            );
            return ExitCode::FAILURE;
        }
    }

    // ── inspect ────────────────────────────────────────────────────
    let inv = inspect::inspect(&dest, is_dir);
    if inv.files.is_empty() {
        eprintln!("grip: {target} has no adoptable files");
        return ExitCode::FAILURE;
    }
    println!(
        "{} {target} — {} file{}, {}{}",
        palette.good("adopting"),
        inv.files.len(),
        if inv.files.len() == 1 { "" } else { "s" },
        inspect::fmt_kib(inv.total_bytes),
        if inv.skipped.is_empty() {
            String::new()
        } else {
            format!(", {} skipped", inv.skipped.len())
        }
    );
    for skipped in &inv.skipped {
        println!(
            "  {} {}",
            palette.warn("skip"),
            palette.dim(&format!("{} ({})", skipped.rel, skipped.reason))
        );
    }
    if inv.total_bytes > inspect::SIZE_WARN_BYTES {
        let largest = inspect::largest(&inv, 3)
            .iter()
            .map(|(rel, size)| format!("{rel} ({})", inspect::fmt_kib(*size)))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  {} this is large for a config repo — largest: {largest}",
            palette.warn("note")
        );
    }

    // ── ask ────────────────────────────────────────────────────────
    let mode = match prompt::ask_mode(mode, palette) {
        Ok(m) => m,
        Err(()) => return ExitCode::FAILURE,
    };
    if mode == generate::MODE_MERGE && is_dir {
        eprintln!("grip: merge owns one block inside a single file — it cannot adopt a directory");
        return ExitCode::FAILURE;
    }
    println!("  {}", palette.dim(&prompt::mode_line(&mode)));

    // ── generate ───────────────────────────────────────────────────
    let rel_files: Vec<String> = inv.files.iter().map(|f| f.rel.clone()).collect();
    let written = [
        format!("configs/{name}/"),
        format!("modules/{name}.ts"),
        format!(
            "hosts/{}.ts",
            host.map(str::to_string).unwrap_or_else(hostname)
        ),
    ];
    let revert = |why: &str| {
        eprintln!("grip: {why}");
        eprintln!(
            "  written so far: {} — remove them (and the `{name}` entry in {}) to abandon",
            written.join(", "),
            written[2]
        );
        ExitCode::FAILURE
    };
    if let Err(e) = generate::write_payload(&repo, &dest, &name, &rel_files, is_dir) {
        return revert(&format!("cannot write payload: {e}"));
    }
    let to = generate::tilde(&dest);
    let module_ts = generate::module_source(
        &name,
        &to,
        rel_files.first().map(String::as_str).unwrap_or("config"),
        &mode,
        is_dir,
    );
    if let Err(e) = std::fs::write(repo.join("modules").join(format!("{name}.ts")), &module_ts) {
        return revert(&format!("cannot write modules/{name}.ts: {e}"));
    }
    let host_name = host.map(str::to_string).unwrap_or_else(hostname);
    let host_rel = format!("hosts/{host_name}.ts");
    let host_path = repo.join(&host_rel);
    if !host_path.is_file() {
        eprintln!("grip: no {host_rel} — create it (or run `grip init`) and add:\n");
        eprintln!("{}", generate::host_snippet(&name));
        return ExitCode::FAILURE;
    }
    let host_src = std::fs::read_to_string(&host_path).unwrap_or_default();
    match generate::update_host(&host_src, &name) {
        Some(updated) => {
            if let Err(e) = std::fs::write(&host_path, updated) {
                return revert(&format!("cannot update {host_rel}: {e}"));
            }
        }
        None => {
            eprintln!(
                "grip: {host_rel} doesn't match the expected defineEnv shape — add this yourself:\n"
            );
            eprintln!("{}", generate::host_snippet(&name));
            return ExitCode::FAILURE;
        }
    }
    println!(
        "  {} configs/{name}/ · modules/{name}.ts · {host_rel}",
        palette.good("wrote")
    );

    // ── plan ───────────────────────────────────────────────────────
    if let Some(code) = trust_gate(&repo) {
        return code;
    }
    let outcome = match eval_repo(&repo, Some(&host_name), palette) {
        Ok(o) => o,
        Err(code) => {
            let _ = revert("generated files don't eval — inspect modules/{name}.ts");
            return code;
        }
    };
    let ir = match crate::commands::check_ir(&outcome.ir_json, palette) {
        Ok(ir) => ir,
        Err(code) => return code,
    };
    let adopting: std::collections::BTreeSet<String> = rel_files
        .iter()
        .map(|rel| generate::tilde(&dest.join(rel)))
        .collect();
    println!("{}", render::diff_section(&ir, &repo, &adopting, palette));
    println!(
        "  {}",
        palette.dim("prior state will be recorded — rollback restores your original files")
    );

    // ── confirm & apply ────────────────────────────────────────────
    if !yes {
        if !std::io::stdin().is_terminal() {
            eprintln!("grip: non-interactive — pass --yes to apply, or run the shown plan first");
            return ExitCode::FAILURE;
        }
        if !prompt::confirm_apply(palette) {
            eprintln!(
                "not applied — repo files stay; remove configs/{name}, modules/{name}.ts and the {host_rel} entry to abandon"
            );
            return ExitCode::FAILURE;
        }
    }
    // generation 0 (0015 §4): on a fresh machine, adopt records the
    // empty baseline first, or the adopt apply IS generation 1 and
    // there is nothing to roll back to
    let home = store::gripsack_home();
    if store::current_generation(&home).is_none()
        && store::list_generations(&home).is_empty()
        && let Err(e) = store::write_manifest(
            &home,
            &store::Generation {
                number: 0,
                modules: Default::default(),
            },
        )
    {
        eprintln!("grip: cannot record the baseline generation: {e}");
        return ExitCode::FAILURE;
    }
    crate::commands::apply_scoped(&repo, adopting, Some(&host_name), None, palette)
}

/// Which module already manages a destination under this path.
fn managed_by(dest: &Path) -> Option<String> {
    let home = store::gripsack_home();
    let n = store::current_generation(&home)?;
    let manifest = store::read_manifest(&home, n).ok()?;
    for (name, state) in &manifest.modules {
        for entry in &state.entries {
            let deployed = expand_home(&entry.to);
            if deployed == *dest || deployed.starts_with(dest) {
                return Some(name.clone());
            }
        }
    }
    None
}
