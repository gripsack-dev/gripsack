use crate::render::{self, Palette};
use gripsack_ir::{Ir, Severity};
use std::path::Path;
use std::process::ExitCode;

/// The eval wire protocol (0011 §5): the frontend emits the IR plus
/// any diagnostics (lint results, frontend-side validation). A bare IR
/// document is still accepted — GRIPSACK_PYTHON may point at an older
/// frontend, and the traceback passthrough covers the rest.
#[derive(serde::Deserialize)]
struct EvalEnvelope {
    ir: serde_json::Value,
    #[serde(default)]
    diagnostics: Vec<gripsack_ir::Diagnostic>,
}

/// Evaluate an env repo's frontend into IR JSON (0005 §4). The core
/// never embeds a runtime — this is a subprocess.
pub fn eval_repo(repo: &Path, host: Option<&str>, palette: Palette) -> Result<String, ExitCode> {
    let env_path = repo.join("env.toml");
    if !env_path.exists() {
        eprintln!(
            "grip: no env.toml in {} — is this an env repo?",
            repo.display()
        );
        return Err(ExitCode::FAILURE);
    }
    let env = match gripsack_config::load_env(&env_path) {
        Ok(env) => env,
        Err(diagnostics) => {
            eprintln!("{}", render::render_diagnostics(&diagnostics, palette));
            return Err(ExitCode::FAILURE);
        }
    };
    if env.env.frontend != gripsack_config::Frontend::Python {
        eprintln!(
            "grip: typescript eval is not available yet — set `frontend = \"python\"` for now"
        );
        return Err(ExitCode::from(2));
    }
    let python = match gripsack_exec::ensure_python(
        &gripsack_store::gripsack_home(),
        &env,
        env!("CARGO_PKG_VERSION"),
    ) {
        Ok(python) => python,
        Err(e) => {
            eprintln!("grip: frontend provisioning failed: {e}");
            eprintln!(
                "hint: set GRIPSACK_PYTHON to a python with `gripsack` installed to bypass provisioning"
            );
            return Err(ExitCode::FAILURE);
        }
    };
    let mut cmd = std::process::Command::new(python);
    cmd.arg("-m").arg("gripsack").arg(repo).current_dir(repo);
    let host = host
        .map(str::to_string)
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "default".into());
    cmd.arg("--host").arg(&host);
    let out = cmd.output().map_err(|e| {
        eprintln!("grip: cannot spawn python3: {e} (see `grip doctor`)");
        ExitCode::FAILURE
    })?;
    let stdout = String::from_utf8(out.stdout).map_err(|_| {
        eprintln!("grip: frontend emitted non-utf8 IR — this is a frontend bug");
        ExitCode::FAILURE
    })?;
    // Envelope (0011 §5): diagnostics render through the same path as
    // the core's own; any error-severity diagnostic fails eval.
    if let Ok(envelope) = serde_json::from_str::<EvalEnvelope>(&stdout) {
        if !envelope.diagnostics.is_empty() {
            eprintln!(
                "{}",
                render::render_diagnostics(&envelope.diagnostics, palette)
            );
        }
        let failed = !out.status.success()
            || envelope
                .diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error);
        if failed {
            return Err(ExitCode::FAILURE);
        }
        return Ok(envelope.ir.to_string());
    }
    if !out.status.success() {
        // Frontend tracebacks are the frontend's domain (0005 §4) —
        // pass them through untouched.
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        eprintln!("grip: frontend eval failed ({host})");
        return Err(ExitCode::FAILURE);
    }
    Ok(stdout)
}

/// Parse + validate IR, rendering diagnostics on failure.
pub fn check_ir(json: &str, palette: Palette) -> Result<Ir, ExitCode> {
    gripsack_ir::check(json).map_err(|diagnostics| {
        for d in &diagnostics {
            tracing::error!(code = d.code.as_ref(), "{}", d.message);
        }
        eprintln!("{}", render::render_diagnostics(&diagnostics, palette));
        ExitCode::FAILURE
    })
}
