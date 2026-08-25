use crate::commands::{check_ir, eval_repo};
use crate::render::Palette;
use owo_colors::OwoColorize;
use std::path::Path;
use std::process::ExitCode;

/// grip check: eval + IR sema + linters, then stop (0011 §9). Zero
/// side effects — no lockfile writes, no store, no staging. The CI
/// gate for env repos and the config-editing loop: exit code = validity.
pub fn check(repo: &Path, host: Option<&str>, palette: Palette) -> ExitCode {
    let json = match eval_repo(repo, host, palette) {
        Ok(j) => j,
        Err(code) => return code,
    };
    match check_ir(&json, palette) {
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
