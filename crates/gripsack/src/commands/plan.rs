use crate::render::{self, Palette};
use std::path::Path;
use std::process::ExitCode;

/// Validate an IR file and show the execution waves (0004 §4, 0007 §5).
#[tracing::instrument(name = "plan", skip(palette), fields(file = %path.display()))]
pub fn plan_ir(path: &Path, palette: Palette) -> ExitCode {
    let json = match std::fs::read_to_string(path) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("grip: cannot read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let ir = match gripsack_ir::check(&json) {
        Ok(ir) => ir,
        Err(diagnostics) => {
            for d in &diagnostics {
                tracing::error!(code = d.code.as_ref(), "{}", d.message);
            }
            eprintln!("{}", render::render_diagnostics(&diagnostics, palette));
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(modules = ir.modules.len(), "ir parsed and validated");
    let host = &ir.host;
    println!(
        "{} {} modules · host {}/{} · tags: {}",
        palette.good("plan:"),
        ir.modules.len(),
        host.os,
        host.arch,
        if host.tags.is_empty() {
            "(none)".to_string()
        } else {
            host.tags.join(", ")
        }
    );
    match gripsack_exec::waves(&ir) {
        Ok(waves) => {
            for (i, wave) in waves.iter().enumerate() {
                println!(
                    "  {} {}",
                    palette.badge(&format!("wave {i}")),
                    wave.join(", ")
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", palette.error(&format!("error: {e}")));
            ExitCode::FAILURE
        }
    }
}

/// Module-scoped view: `grip plan --ir FILE <module>` (0007 §5).
pub fn plan_module(path: &Path, name: &str, palette: Palette) -> ExitCode {
    let json = match std::fs::read_to_string(path) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("grip: cannot read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let ir = match gripsack_ir::check(&json) {
        Ok(ir) => ir,
        Err(diagnostics) => {
            eprintln!("{}", render::render_diagnostics(&diagnostics, palette));
            return ExitCode::FAILURE;
        }
    };
    if !ir.modules.contains_key(name) {
        eprintln!(
            "grip: no module {name:?} in the graph (have: {})",
            ir.modules.keys().cloned().collect::<Vec<_>>().join(", ")
        );
        return ExitCode::FAILURE;
    }
    let waves = gripsack_exec::waves(&ir).unwrap_or_default();
    println!("{}", render::render_module(&ir, name, &waves, palette));
    ExitCode::SUCCESS
}
