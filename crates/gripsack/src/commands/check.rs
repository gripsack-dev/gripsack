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
            let output_count = ir
                .workspace
                .as_ref()
                .map(|w| w.outputs.len())
                .or_else(|| ir.workspace_v4.as_ref().map(|w| w.outputs.len()));
            if let Some(count) = output_count {
                let host = &ir.host;
                println!(
                    "{} {} named outputs · host {}/{}",
                    palette.good("check: ok"),
                    count,
                    host.os,
                    host.arch
                );
                if let Some(workspace) = &ir.workspace {
                    for output in &workspace.outputs {
                        println!("  {} ({})", output.name(), output.kind());
                    }
                } else if let Some(workspace) = &ir.workspace_v4 {
                    for output in &workspace.outputs {
                        println!("  {} ({})", output.name(), output.kind());
                    }
                }
                return ExitCode::SUCCESS;
            }
            // physical destination uniqueness (0030 §P0-1): reads
            // only, no side effects — two spellings of one directory
            // entry are a check-time error, rendered like any sema
            // diagnostic (code, spans, help)
            match gripsack_exec::expand::expand_all(&ir.modules).and_then(|plans| {
                gripsack_exec::expand::check_physical_uniqueness(&ir.modules, &plans)
            }) {
                Ok(()) => {}
                Err(gripsack_exec::ctx::ExecError::Gate(d)) => {
                    eprintln!("{}", crate::render::render_diagnostics(&[d], palette));
                    return ExitCode::FAILURE;
                }
                Err(e) => {
                    eprintln!("grip: {e}");
                    return ExitCode::FAILURE;
                }
            }
            match gripsack_exec::inspect_known_layouts(&ir, repo, &outcome.host) {
                Ok(layouts) => {
                    for (module, evidence) in layouts {
                        if let Some(summary) = evidence.summary() {
                            println!("  {module}: {summary}");
                        }
                    }
                }
                Err(error) => {
                    eprintln!("grip: {error}");
                    return ExitCode::FAILURE;
                }
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
