//! A declaration spelling is not yet a filesystem path (0036, 0041).
//! Preview and deployment use this same substitution/join boundary.

mod acquire;
mod overlay;
pub(crate) mod preflight;
pub(crate) use acquire::{FetchInputs, fetch, publish};
pub(crate) use overlay::Overlay;

use std::path::{Path, PathBuf};

pub(crate) struct PayloadSource {
    pub relative: String,
    pub path: PathBuf,
}

pub(crate) fn payload_source(
    root: &Path,
    declaration: &str,
    version: Option<&str>,
) -> Result<PayloadSource, gripsack_fetch::PlaceholderError> {
    let relative = gripsack_fetch::placeholders::payload_path(declaration, version)?;
    let path = root.join(&relative);
    Ok(PayloadSource { relative, path })
}
