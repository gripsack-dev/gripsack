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
