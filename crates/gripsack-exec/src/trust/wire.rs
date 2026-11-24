//! Informational git provenance and repository-identity validation.
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Component, Path},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitProvenance {
    pub remote: Option<String>,
    pub commit: Option<String>,
}

impl GitProvenance {
    /// Admit informational native-query results without retaining credentials.
    pub fn from_git(remote: Option<&str>, commit: Option<String>) -> Self {
        Self {
            remote: remote.and_then(super::audit::redact_remote),
            commit: commit.filter(|value| {
                matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            }),
        }
    }
}

pub(super) fn validate_repository(repository: &str) -> io::Result<()> {
    let path = Path::new(repository);
    if !path.is_absolute()
        || repository.contains('\0')
        || repository.contains("//")
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(invalid(
            "source approval repository is not an absolute canonical path",
        ));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
