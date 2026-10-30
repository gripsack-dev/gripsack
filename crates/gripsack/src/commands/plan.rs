use crate::render::{self, Palette};
use std::path::Path;
use std::process::ExitCode;

fn ir_source_root(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Validate current IR and preview the selected workspace profiles.
#[tracing::instrument(name = "plan", skip(palette), fields(file = %path.display()))]
pub fn plan_ir(path: &Path, selected: &[String], palette: Palette, structured: bool) -> ExitCode {
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
            eprintln!(
                "{}",
                render::render_diagnostics_bounded(&diagnostics, palette, ir_source_root(path))
            );
            return ExitCode::FAILURE;
        }
    };
    if let Err(code) = crate::commands::reject_workspace_execution(
        &ir,
        gripsack_ir::workspace::WorkspaceOperation::Plan,
        &mut crate::render::DiagnosticSink::terminal(palette, ir_source_root(path)),
    ) {
        return code;
    }
    if structured {
        return super::review::plan(
            &ir,
            &gripsack_exec::Repository::direct(ir_source_root(path).to_path_buf()),
            selected,
            Default::default(),
            true,
        );
    }
    native_preview(&ir, ir_source_root(path), selected, palette)
}

fn native_preview(
    ir: &gripsack_ir::Ir,
    repo: &Path,
    selected: &[String],
    palette: Palette,
) -> ExitCode {
    match render::diff_section(
        ir,
        &gripsack_exec::Repository::direct(repo.to_path_buf()),
        &Default::default(),
        palette,
        selected,
        Default::default(),
    ) {
        Ok(section) => {
            println!("{section}");
            ExitCode::SUCCESS
        }
        Err(gripsack_exec::ExecError::Gate(diagnostic)) => {
            eprintln!(
                "{}",
                render::render_diagnostics_bounded(&[diagnostic], palette, repo)
            );
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("grip: cannot compute the preview: {error}");
            ExitCode::FAILURE
        }
    }
}
