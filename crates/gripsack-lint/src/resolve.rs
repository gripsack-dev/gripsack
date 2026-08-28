//! Registration → executable (0010 §3): `path` wins; `package` is a
//! plugin-store ref (`owner/repo@tag`) resolved from the store.

use gripsack_config::LinterSection;
use gripsack_ir::{Diagnostic, Severity};
use std::path::PathBuf;

use crate::MISSING_EXECUTABLE;

/// Resolve the linter executable for one registration.
pub(crate) fn resolve_exe(
    name: &str,
    reg: &LinterSection,
    reg_label: impl Fn() -> Diagnostic,
) -> Result<PathBuf, Diagnostic> {
    match (&reg.path, &reg.package) {
        (Some(_), Some(_)) => Err(reg_label().with_help("pick one")),
        (Some(path), None) => Ok(PathBuf::from(path)),
        (None, Some(package)) => {
            // a repo ref (owner/repo@tag) means a provisioned plugin
            // binary from the store (0012 §move-2); any other form is
            // a leftover pip-wheel pin from the python-frontend era
            if gripsack_fetch::plugins::parse_ref(package).is_some() {
                let store =
                    gripsack_fetch::plugins::PluginStore::new(&gripsack_store::gripsack_home());
                let exe = format!("griplint-{name}");
                return match store.current_binary(&exe) {
                    Some(bin) => Ok(bin),
                    None => Err(Diagnostic {
                        code: std::borrow::Cow::Borrowed(MISSING_EXECUTABLE),
                        severity: Severity::Error,
                        message: format!(
                            "linter {name:?} is declared as {package:?} but {exe} is not \
                             provisioned — run any grip command to install declared plugins"
                        ),
                        labels: Vec::new(),
                        help: None,
                    }),
                };
            }
            Err(Diagnostic {
                code: std::borrow::Cow::Borrowed(MISSING_EXECUTABLE),
                severity: Severity::Error,
                message: format!(
                    "linter {name:?} is registered as package {package:?} — only \
                     plugin-store refs (owner/repo@tag) provision; the pip-wheel form \
                     died with the python frontend (plan/0013 D1)"
                ),
                labels: Vec::new(),
                help: Some("use `package = \"owner/repo@tag\"` or an explicit `path`".into()),
            })
        }
        (None, None) => Err(reg_label()),
    }
}
