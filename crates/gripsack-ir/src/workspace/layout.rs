//! Relocatable versus fixed-prefix package identity (0052 A1-02).
//! The prefix is declared for cross-target admission, not inferred
//! from a local generation or the machine running `grip check`.

use serde::{Deserialize, Serialize};

/// Absolute POSIX install destination, checked at IR admission. This
/// domain value is not interchangeable with an arbitrary host path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstallPrefix(pub String);

impl InstallPrefix {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_safe(&self) -> bool {
        let path = self.as_str();
        path.len() > 1
            && path.starts_with('/')
            && !path.contains('\0')
            && path[1..]
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
    }
}

/// Prefix-bound results have a separate materialization identity;
/// they cannot be treated as relocatable by a consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PackageLayout {
    Relocatable,
    FixedPrefix { prefix: InstallPrefix },
}
