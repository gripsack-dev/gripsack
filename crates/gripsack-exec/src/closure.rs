//! Build-step environments (0039). Graph membership belongs to the IR; order
//! comes from the executor's validated DAG. No second scheduler or cycle fallback.

use crate::ctx::ExecError;
use gripsack_store::ModuleState;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Command;

/// Prepared once per module, used by every produce-phase process. The values
/// are store roots, not declarations or destination paths (0036).
#[derive(Debug, Default)]
pub(crate) struct BuildEnv {
    exports: Vec<(String, PathBuf)>,
}

impl BuildEnv {
    pub(crate) fn compose(
        names: &[&str],
        finished: &BTreeMap<String, ModuleState>,
    ) -> Result<Self, ExecError> {
        let exports = names
            .iter()
            .map(|name| {
                let state = finished.get(*name).ok_or_else(|| ExecError::Step {
                    module: (*name).to_owned(),
                    step: "build closure".into(),
                    detail: "dependency has not published its store path".into(),
                })?;
                Ok((
                    gripsack_ir::dependencies::build_dep_var(name),
                    state.store_path.clone(),
                ))
            })
            .collect::<Result<_, ExecError>>()?;
        Ok(Self { exports })
    }

    /// Step overrides are already on `command`. Preserve that PATH (or the
    /// ambient PATH if absent), prepending the closure in graph order. Keep
    /// OsStrings end to end; non-UTF-8 HOME/PATH must not silently lose bytes.
    pub(crate) fn apply(&self, command: &mut Command) -> Result<(), String> {
        if self.exports.is_empty() {
            return Ok(());
        }
        let explicit = command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("PATH"))
            .map(|(_, value)| value.map(OsStr::to_os_string));
        let ambient: Option<OsString> = explicit.unwrap_or_else(|| std::env::var_os("PATH"));
        let bins = self.exports.iter().map(|(_, root)| root.join("bin"));
        let remainder = ambient.iter().flat_map(std::env::split_paths);
        let path = std::env::join_paths(bins.chain(remainder))
            .map_err(|e| format!("cannot compose build closure PATH: {e}"))?;
        command
            .env("PATH", path)
            .envs(self.exports.iter().map(|(name, root)| (name, root)));
        Ok(())
    }

    pub(crate) fn into_paths(self) -> Vec<PathBuf> {
        self.exports.into_iter().map(|(_, path)| path).collect()
    }
}
