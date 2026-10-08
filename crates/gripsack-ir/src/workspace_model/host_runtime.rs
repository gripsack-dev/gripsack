//! Package-scoped GNU loader policy. These are reviewed host dependencies,
//! never artifact content, evaluator grants, or producer recipe inputs.
use serde::{Deserialize, Deserializer, Serialize};
use std::path::Path;

pub(super) fn deserialize_policy<'de, D: Deserializer<'de>>(
    decoder: D,
) -> Result<Option<HostRuntimeRequirements>, D::Error> {
    let policy = HostRuntimeRequirements::deserialize(decoder)?;
    policy.validate().map_err(serde::de::Error::custom)?;
    Ok(Some(policy))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRuntimeRequirements {
    /// Explicit library search roots, in declared search order.
    pub library_directories: Vec<HostLibraryDirectory>,
}

impl HostRuntimeRequirements {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.library_directories.is_empty() {
            return Err("host runtime requires at least one library directory");
        }
        for (index, directory) in self.library_directories.iter().enumerate() {
            if self.library_directories[..index].contains(directory) {
                return Err("host runtime library directories must be unique");
            }
        }
        Ok(())
    }
}

/// An absolute, normalized Linux directory without loader-token expansion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct HostLibraryDirectory(String);

impl HostLibraryDirectory {
    pub fn new(value: String) -> Result<Self, &'static str> {
        if !value.starts_with('/')
            || value == "/"
            || value
                .bytes()
                .any(|byte| byte < 32 || byte == 127 || matches!(byte, b'\\' | b':' | b';' | b'$'))
            || value[1..]
                .split('/')
                .any(|part| matches!(part, "" | "." | ".."))
        {
            return Err(
                "host library directory must be an absolute normalized Linux path without loader tokens",
            );
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<Path> for HostLibraryDirectory {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl<'de> Deserialize<'de> for HostLibraryDirectory {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(decoder)?).map_err(serde::de::Error::custom)
    }
}
