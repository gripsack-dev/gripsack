//! Registration → executable (0010 §3): `path` wins; `package`
//! resolves the console script next to the frontend python.

use gripsack_config::LinterSection;
use gripsack_ir::{Diagnostic, Severity};
use std::path::{Path, PathBuf};

use crate::MISSING_EXECUTABLE;

/// Resolve the linter executable for one registration.
pub(crate) fn resolve_exe(
    name: &str,
    reg: &LinterSection,
    frontend_python: Option<&Path>,
    reg_label: impl Fn() -> Diagnostic,
) -> Result<PathBuf, Diagnostic> {
    match (&reg.path, &reg.package) {
        (Some(_), Some(_)) => Err(reg_label().with_help("pick one")),
        (Some(path), None) => Ok(PathBuf::from(path)),
        (None, Some(package)) => {
            // a repo ref (owner/repo@tag) means a provisioned plugin
            // binary from the store (0012 §move-2) — the wheel meaning
            // stays for bare package names
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
            let exe = frontend_python
                .and_then(|p| p.parent().map(|d| d.join(format!("griplint-{name}"))));
            match exe {
                Some(exe) if exe.exists() => Ok(exe),
                _ => Err(Diagnostic {
                    code: std::borrow::Cow::Borrowed(MISSING_EXECUTABLE),
                    severity: Severity::Error,
                    message: format!(
                        "linter {name:?} is registered as package {package:?} but \
                         griplint-{name} was not found next to the frontend python"
                    ),
                    labels: Vec::new(),
                    help: Some(
                        "provisioning installs registered packages; a GRIPSACK_PYTHON \
                         bypass means you must install the linter yourself"
                            .into(),
                    ),
                }),
            }
        }
        (None, None) => Err(reg_label()),
    }
}
