//! The linter protocol host (0011, 0012 §move-1): the CORE drives
//! linters, exactly like fetchers — one JSON request on stdin, NDJSON
//! diagnostics and one response on stdout, death never silent.
//!
//! Layout: `resolve` (registration → executable), `exchange` (the
//! protocol conversation), `versions` (lockfile pins), and `run` below
//! (the walk over IR modules). Semantics ported from the frontend's
//! lint.py, unchanged:
//! - registration: `path` wins over `package`
//! - a label-less plugin diagnostic gets the module-callsite label
//! - crash-class codes (E99/E02) are ALWAYS warning severity — a
//!   plugin's self-reported severity for its own crash is not evidence

mod exchange;
mod resolve;
mod versions;

use gripsack_config::LinterSection;
use gripsack_ir::{Diagnostic, Ir};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const UNREGISTERED_LINTER: &str = "E501";
pub const BAD_REGISTRATION: &str = "E502";
pub const MISSING_EXECUTABLE: &str = "E503";

/// Lint every module that declares `lint` against the registry.
/// Returns diagnostics; error severity fails the calling command.
pub fn run(
    ir: &Ir,
    linters: &BTreeMap<String, LinterSection>,
    repo: &Path,
    host: Option<&str>,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let versions = versions::tool_versions(repo, host);
    for (name, module) in &ir.modules {
        let Some(lint) = &module.lint else { continue };
        let mut paths: Vec<PathBuf> = module
            .config
            .iter()
            .map(|e| repo.join(&e.from))
            .filter(|p| p.is_file())
            .collect();
        paths.sort();
        let Some(reg) = linters.get(lint) else {
            // no registration: a built-in pack runs in-process — the
            // engine in the crate (0012 §move-3); no provisioning, no
            // venv, no lifecycle for first-party linters
            if let Some(pack) = griplint::pack_for(lint) {
                for path in paths {
                    let Ok(text) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    out.extend(griplint::checks::lint_file(
                        &pack,
                        &path.to_string_lossy(),
                        &text,
                        versions.get(name).map(String::as_str),
                    ));
                }
                continue;
            }
            out.push(
                Diagnostic::error(
                    UNREGISTERED_LINTER,
                    format!(
                        "module {name:?} lints with {lint:?}, which is not registered and has no built-in pack"
                    ),
                )
                .with_label(module.span.clone(), "lint requested here")
                .with_help(format!("add [linters.{lint}] to env.toml (0010 §3)")),
            );
            continue;
        };
        let reg_label = || {
            Diagnostic::error(
                BAD_REGISTRATION,
                format!(
                    "linter {lint:?} needs `package` or `path` in env.toml [linters] (not both)"
                ),
            )
        };
        let exe = match resolve::resolve_exe(lint, reg, reg_label) {
            Ok(exe) => exe,
            Err(mut d) => {
                if let Some(span) = &module.span {
                    d = d.with_label(Some(span.clone()), "lint requested here");
                }
                out.push(d);
                continue;
            }
        };
        out.extend(exchange::run_linter(
            &exe,
            lint,
            &paths,
            versions.get(name).map(String::as_str),
            name,
            &module.span,
        ));
    }
    out
}
