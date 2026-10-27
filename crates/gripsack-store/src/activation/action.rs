//! Durable activation actions, including retained native hooks. Workspace
//! commands are admitted only by version-two manifests and activation plans.
use gripsack_process::Sha256Digest;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActivationAction {
    Service {
        name: String,
        user: bool,
    },
    Fonts,
    DesktopEntry,
    CustomShell {
        script: String,
    },
    WorkspaceHook {
        context: PathBuf,
        sha256: Sha256Digest,
    },
}

impl ActivationAction {
    pub(crate) fn requires_v2(&self) -> bool {
        matches!(self, Self::WorkspaceHook { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_action_bytes_preserve_durable_identity() {
        for bytes in [
            r#"{"kind":"fonts"}"#,
            r#"{"kind":"desktop_entry"}"#,
            r#"{"kind":"service","name":"demo.service","user":false}"#,
            r#"{"kind":"service","name":"demo.service","user":true}"#,
            r#"{"kind":"custom_shell","script":"printf exact"}"#,
        ] {
            let stored: ActivationAction = serde_json::from_str(bytes).unwrap();
            assert_eq!(serde_json::to_string(&stored).unwrap(), bytes);
        }
    }

    #[test]
    fn stored_action_reader_is_closed() {
        for invalid in [
            r#"{"kind":"service","name":"demo"}"#,
            r#"{"kind":"custom_shell","script":"true","extra":true}"#,
            r#"{"kind":"workspace_hook","context":"/tmp/context"}"#,
            r#"{"kind":"workspace_hook","context":"/tmp/context","sha256":"not-a-digest"}"#,
        ] {
            assert!(serde_json::from_str::<ActivationAction>(invalid).is_err());
        }
    }
}
