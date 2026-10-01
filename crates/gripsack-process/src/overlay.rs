//! Repository-owned process overlay: explicitly admitted values layered over
//! the operator snapshot for project consumer processes (run/shell/task).
//! The operator environment is never mutated into carrying repository data;
//! the merge happens once, here, with deterministic precedence:
//!
//! 1. overlay search prefix directories (package commands first, then
//!    declared PATH directories), then the operator's PATH;
//! 2. overlay entries replace operator values for the same key;
//! 3. remaining operator entries pass through (already role-filtered).
//!
//! PATH and the evaluator-private DENO_DIR cannot be declared as overlay
//! entries: PATH is composed from the package selection plus an explicit
//! declared suffix, and the reservation keys belong to lifecycle identity.
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io,
    os::unix::ffi::OsStrExt,
    path::PathBuf,
};

const RESERVED_KEYS: [&str; 5] = [
    "PATH",
    "LD_LIBRARY_PATH",
    "DENO_DIR",
    "GRIPSACK_ACTIVATION_INTENT_ID",
    "GRIPSACK_ACTIVATION_ATTEMPT",
];

#[derive(Debug, Clone, Default)]
pub struct EnvironmentOverlay {
    entries: BTreeMap<OsString, OsString>,
    search_prefix: Vec<PathBuf>,
    library_prefix: Vec<PathBuf>,
}

impl EnvironmentOverlay {
    /// Admit repository-declared values. Keys must be nonempty and free of
    /// `=`/NUL; reserved keys are rejected rather than silently overridden.
    /// Search/library prefix directories must be absolute and keep
    /// declaration order.
    pub fn admit(
        entries: impl IntoIterator<Item = (OsString, OsString)>,
        search_prefix: impl IntoIterator<Item = PathBuf>,
        library_prefix: impl IntoIterator<Item = PathBuf>,
    ) -> io::Result<Self> {
        let mut admitted = BTreeMap::new();
        for (key, value) in entries {
            let name = key.to_str().ok_or_else(invalid_key)?;
            if name.is_empty() || name.contains('=') {
                return Err(invalid_key());
            }
            if RESERVED_KEYS.contains(&name) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("environment overlay cannot declare reserved key {name:?}"),
                ));
            }
            if key.as_bytes().contains(&0) || value.as_bytes().contains(&0) {
                return Err(invalid_key());
            }
            admitted.insert(key, value);
        }
        let search_prefix = absolute_prefix(search_prefix)?;
        let library_prefix = absolute_prefix(library_prefix)?;
        Ok(Self {
            entries: admitted,
            search_prefix,
            library_prefix,
        })
    }

    /// The admitted per-key values, sorted — task command environments layer
    /// their own additions through a fresh admission.
    pub fn entries(&self) -> &BTreeMap<OsString, OsString> {
        &self.entries
    }
    pub fn search_prefix(&self) -> &[PathBuf] {
        &self.search_prefix
    }
    pub fn library_prefix(&self) -> &[PathBuf] {
        &self.library_prefix
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.search_prefix.is_empty() && self.library_prefix.is_empty()
    }

    /// Merge over the operator's role-filtered entries. The result is a
    /// sorted key/value view: overlay wins on shared keys; PATH is the
    /// overlay search prefix followed by the operator's search path.
    pub(crate) fn merge<'a>(
        &'a self,
        operator: impl Iterator<Item = (&'a OsStr, &'a OsStr)>,
    ) -> BTreeMap<OsString, OsString> {
        let mut merged: BTreeMap<OsString, OsString> = operator
            .map(|(key, value)| (key.to_os_string(), value.to_os_string()))
            .collect();
        for (key, value) in &self.entries {
            merged.insert(key.clone(), value.clone());
        }
        if !self.search_prefix.is_empty() {
            let operator_path = merged
                .get(OsStr::new("PATH"))
                .cloned()
                .unwrap_or_else(|| OsString::from("/usr/bin:/bin"));
            let mut path = OsString::new();
            for (index, directory) in self.search_prefix.iter().enumerate() {
                if index > 0 {
                    path.push(":");
                }
                path.push(directory.as_os_str());
            }
            if !operator_path.is_empty() {
                path.push(":");
                path.push(&operator_path);
            }
            merged.insert(OsString::from("PATH"), path);
        }
        if !self.library_prefix.is_empty() {
            let mut path = OsString::new();
            for (index, directory) in self.library_prefix.iter().enumerate() {
                if index > 0 {
                    path.push(":");
                }
                path.push(directory.as_os_str());
            }
            merged.insert(OsString::from("LD_LIBRARY_PATH"), path);
        }
        merged
    }
}

fn absolute_prefix(
    directories: impl IntoIterator<Item = PathBuf>,
) -> io::Result<Vec<PathBuf>> {
    let mut prefix = Vec::new();
    for directory in directories {
        if !directory.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "environment overlay prefix directory must be absolute",
            ));
        }
        if directory.as_os_str().as_bytes().contains(&0) {
            return Err(invalid_key());
        }
        if !prefix.contains(&directory) {
            prefix.push(directory);
        }
    }
    Ok(prefix)
}

fn invalid_key() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "environment overlay keys/values must be nonempty UTF-8 without '=' or NUL",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(key: &str, value: &str) -> (OsString, OsString) {
        (OsString::from(key), OsString::from(value))
    }

    #[test]
    fn overlay_replaces_shared_keys_and_prepends_search_directories() {
        let overlay = EnvironmentOverlay::admit(
            [pair("EDITOR", "hx"), pair("LANG", "declared")],
            [PathBuf::from("/pkg/bin"), PathBuf::from("/declared/bin")],
            [PathBuf::from("/pkg/lib")],
        )
        .unwrap();
        let operator = OperatorEnvironment::admit([
            pair("PATH", "/usr/bin:/bin"),
            pair("LANG", "operator"),
            pair("HOME", "/home/op"),
        ])
        .unwrap();
        let merged = overlay.merge(operator.entries(ProcessRole::Task));
        assert_eq!(merged[OsStr::new("EDITOR")].as_os_str(), OsStr::new("hx"));
        assert_eq!(merged[OsStr::new("LANG")].as_os_str(), OsStr::new("declared"));
        assert_eq!(merged[OsStr::new("HOME")].as_os_str(), OsStr::new("/home/op"));
        assert_eq!(
            merged[OsStr::new("PATH")].as_os_str(),
            OsStr::new("/pkg/bin:/declared/bin:/usr/bin:/bin")
        );
        assert_eq!(
            merged[OsStr::new("LD_LIBRARY_PATH")].as_os_str(),
            OsStr::new("/pkg/lib")
        );
    }

    #[test]
    fn reserved_keys_and_relative_search_directories_are_rejected() {
        for key in RESERVED_KEYS {
            assert!(EnvironmentOverlay::admit([pair(key, "x")], [], []).is_err());
        }
        assert!(EnvironmentOverlay::admit([pair("OK", "x")], [PathBuf::from("relative/bin")], []).is_err());
        assert!(EnvironmentOverlay::admit([pair("A=B", "x")], [], []).is_err());
        assert!(EnvironmentOverlay::admit([pair("", "x")], [], []).is_err());
    }

    use super::super::{OperatorEnvironment, ProcessRole};
}
