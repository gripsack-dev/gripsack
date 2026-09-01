//! E116/E117 — names that flow into paths and shell must be shaped.
//!
//! Module names become store path segments (`<hash>-<name>`) and
//! manifest keys; a name like `x/../../pwned` would walk out of the
//! store. Env var names are interpolated unquoted into `export`
//! lines in profile.sh — the value side is quoted, the name side
//! cannot be, so it is validated instead.

use crate::diagnostic::{Diagnostic, codes};
use crate::model::Ir;

/// A safe module name: nonempty, no path separators, no `.`/`..`,
/// no `:` (module:step refs split on it), no control characters.
pub fn module_name_ok(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !name.contains("..")
}

/// A shell-safe env var name: POSIX identifier.
fn env_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    for (name, module) in &ir.modules {
        if !module_name_ok(name) {
            diagnostics.push(
                Diagnostic::error(
                    codes::INVALID_MODULE_NAME,
                    format!(
                        "invalid module name {name:?}: use letters, digits, '_', '-', '.' \
                         (no path separators, no ':', no leading '.')"
                    ),
                )
                .with_label(module.span.clone(), "module declared here"),
            );
        }
        for var in &module.env {
            if !env_name_ok(&var.name) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::INVALID_ENV_NAME,
                        format!(
                            "module {name:?}: env var name {:?} is not a shell identifier \
                             (letters, digits, '_', leading letter or '_')",
                            var.name
                        ),
                    )
                    .with_label(module.span.clone(), "module declaring this env var"),
                );
            }
        }
    }
}
