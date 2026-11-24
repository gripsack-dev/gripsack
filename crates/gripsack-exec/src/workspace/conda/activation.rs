//! Hermetic activation environment of an installed conda prefix (#405).
//!
//! conda sources `etc/conda/activate.d/*.sh` into the user's shell. Gripsack
//! never runs them: the installed prefix is content-addressed and validated
//! against the original archives, so its activation scripts are fixed bytes,
//! and `gripsack_conda::activation` interprets their export-only effect as a
//! pure function of those bytes and the prefix path.

use super::CondaRuntimeReceipt;
use gripsack_conda::activation::{MAX_SCRIPT_BYTES, evaluate};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

/// Directories whose `*.sh` files conda sources on activation. Evaluated
/// hermetically, never executed.
pub(super) const ACTIVATE_DIRECTORIES: [&str; 2] = ["etc/conda/activate.d", "etc/activate.d"];
/// Deactivation scripts never run (there is no deactivation), so their
/// presence is inert and their content is not interpreted.
pub(super) const DEACTIVATE_DIRECTORIES: [&str; 2] = ["etc/conda/deactivate.d", "etc/deactivate.d"];

fn failure(detail: impl std::fmt::Display) -> String {
    format!("Conda activation: {detail}")
}

fn read_scripts(root: &Path) -> Result<Vec<(String, String)>, String> {
    let prefix = gripsack_host::fs::open(root).map_err(failure)?;
    let mut scripts = Vec::new();
    for directory in ACTIVATE_DIRECTORIES {
        let mut handle = None;
        let mut current = None;
        for (index, component) in directory.split('/').enumerate() {
            let parent = current.as_ref().unwrap_or(&prefix);
            match gripsack_host::fs::open_dir_nofollow(parent, Path::new(component)) {
                Ok(next) => current = Some(next),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(failure(format!("{directory}: {error}"))),
            }
            if index + 1 == directory.split('/').count() {
                handle = current.take();
            }
        }
        let Some(handle) = handle else { continue };
        let mut names = Vec::new();
        for entry in handle.entries().map_err(failure)? {
            let entry = entry.map_err(failure)?;
            names.push(
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| failure(format!("non-UTF-8 name in {directory}")))?,
            );
        }
        names.sort();
        for name in names {
            let path = format!("{directory}/{name}");
            if !name.ends_with(".sh") {
                return Err(failure(format!("{path}: only `*.sh` scripts are admitted")));
            }
            let file = gripsack_host::fs::open_file_nofollow(&handle, Path::new(&name))
                .map_err(|error| failure(format!("{path}: {error}")))?;
            let metadata = file
                .metadata()
                .map_err(|error| failure(format!("{path}: {error}")))?;
            if !metadata.is_file() || metadata.len() > MAX_SCRIPT_BYTES as u64 {
                return Err(failure(format!("{path}: not a bounded regular file")));
            }
            let mut text = String::new();
            file.take(MAX_SCRIPT_BYTES as u64 + 1)
                .read_to_string(&mut text)
                .map_err(|error| failure(format!("{path}: {error}")))?;
            scripts.push((path, text));
        }
    }
    Ok(scripts)
}

/// The variables a prefix's activation scripts export, ordered by name.
/// Ambient variables are treated as unset, so the result is deterministic.
pub(in crate::workspace) fn activation_environment(
    receipt: &CondaRuntimeReceipt,
) -> Result<BTreeMap<String, String>, String> {
    let root = &receipt.final_prefix;
    let prefix = root
        .to_str()
        .ok_or_else(|| failure("prefix path is not UTF-8"))?;
    let scripts = read_scripts(root)?;
    evaluate(
        scripts
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str())),
        prefix,
    )
    .map_err(|error| failure(format!("unsupported activation script {error}")))
}
