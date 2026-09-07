//! A declaration spelling is not yet a filesystem path (0036, 0041).
//! Preview and deployment use this same substitution/join boundary.

use std::path::{Path, PathBuf};

pub(crate) struct PayloadSource {
    pub relative: String,
    pub path: PathBuf,
}

pub(crate) fn payload_source(
    root: &Path,
    declaration: &str,
    version: Option<&str>,
) -> PayloadSource {
    let expanded = gripsack_fetch::expand_platform(declaration);
    let relative = match version {
        Some(version) => expanded.replace("{version}", version),
        None => expanded,
    };
    let path = root.join(&relative);
    PayloadSource { relative, path }
}
