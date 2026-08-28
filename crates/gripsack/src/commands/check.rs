use crate::commands::{check_ir, eval_repo, run_lints, trust_gate};
use crate::render::Palette;
use owo_colors::OwoColorize;
use std::path::Path;
use std::process::ExitCode;

/// grip check: eval + IR sema + linters, then stop (0011 §9). Zero
/// side effects — no lockfile writes, no store, no staging. The CI
/// gate for env repos and the config-editing loop: exit code = validity.
pub fn check(repo: &Path, host: Option<&str>, palette: Palette) -> ExitCode {
    if let Some(code) = trust_gate(repo) {
        return code;
    }
    let outcome = match eval_repo(repo, host, palette) {
        Ok(o) => o,
        Err(code) => return code,
    };
    match check_ir(&outcome.ir_json, palette)
        .and_then(|ir| crate::commands::validate_sources(&ir, repo, palette).map(|_| ir))
        .and_then(|ir| run_lints(&ir, &outcome, repo, host, palette).map(|_| ir))
    {
        Ok(ir) => {
            let host = &ir.host;
            println!(
                "{} {} modules · host {}/{} · tags: {}",
                "check: ok".green().bold(),
                ir.modules.len(),
                host.os,
                host.arch,
                if host.tags.is_empty() {
                    "(none)".to_string()
                } else {
                    host.tags.join(", ")
                }
            );
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}
