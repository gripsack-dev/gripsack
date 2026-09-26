//! One admitted host entrypoint/lockfile identity (0048 §1.3).
//!
//! A host name is a single path component, never a path. The same
//! safe-segment grammar already governs module names (E116), but the
//! types stay distinct: a module cannot be passed where a host selects
//! `hosts/<host>.ts` or `locks/<host>.lock`.

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
        if !crate::sema::names::module_name_ok(&value) {
            return Err(Diagnostic::error(
                codes::INVALID_HOST_NAME,
                format!(
                    "invalid host name {value:?}: a host selects one entrypoint and lockfile name, never a path"
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
