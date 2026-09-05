use crate::commands::{eval_repo, trust_gate, validated_ir};
use crate::render::Palette;
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
    match validated_ir(&outcome, repo, host, palette) {
        Ok(ir) => {
            // physical destination uniqueness (0030 §P0-1): reads
            // only, no side effects — two spellings of one directory
            // entry are a check-time error
            let steps = gripsack_exec::expand::expand_all(&ir.modules);
            if let Err(e) = gripsack_exec::expand::check_physical_uniqueness(&steps) {
                eprintln!("grip: {e}");
                return ExitCode::FAILURE;
            }
            let host = &ir.host;
            println!(
                "{} {} modules · host {}/{} · tags: {}",
                palette.good("check: ok"),
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
