//! One admitted workspace host identity, never a path or alternate entrypoint.

use crate::diagnostic::{Diagnostic, codes};
use std::fmt;

/// A validated host selector. The field is private so direct executor
/// callers cannot construct a traversal host and bypass CLI admission.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostName(String);

impl HostName {
    /// Admit an explicit flag, env.toml default or sanitized machine
    /// hostname before any evaluator provisioning or host-derived I/O.
    /// An owned String is reused without copying on the successful path.
    pub fn parse(value: impl Into<String>) -> Result<Self, Diagnostic> {
        let value = value.into();
        if value.is_empty()
            || value.starts_with('.')
            || value.contains("..")
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        {
            return Err(Diagnostic::error(
                codes::INVALID_HOST_NAME,
                format!(
                    "invalid host name {value:?}: a workspace host identity is a safe name, never a path"
                ),
            )
            .with_help(
                "use ASCII letters, digits, '_', '-', or '.' after the first character; no empty name, separators, or '..'",
            ));
        }
        Ok(Self(value))
    }

    /// The selected host's spelling for the versioned frontend inputs
    /// wire. Only the admitted value crosses that serialization boundary.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Transfer an admitted selector to the owned eval boundary
    /// without copying; eval re-admits it after config layering.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for HostName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::HostName;
    use crate::codes;

    #[test]
    fn rejects_path_shaped_hosts_before_they_can_name_files() {
        for invalid in [
            "",
            "../modules/role",
            "/tmp/victim",
            "a/b",
            r"a\b",
            "a..b",
            ".hidden",
            "a:b",
            "ümlaut",
        ] {
            let rejected = HostName::parse(invalid).unwrap_err();
            assert_eq!(rejected.code, codes::INVALID_HOST_NAME, "{invalid:?}");
        }
        for valid in ["workstation", "role.dev", "test-host_2"] {
            let admitted = HostName::parse(valid).unwrap();
            assert_eq!(admitted.as_str(), valid);
        }
    }
}
