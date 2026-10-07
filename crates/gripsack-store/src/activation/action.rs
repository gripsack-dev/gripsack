//! Retained actions are distinct from historical declaration IR. Workspace
//! commands are admitted only by version-two manifests and activation plans.
use gripsack_process::Sha256Digest;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActivationAction {
    Service { name: String, user: bool },
    Fonts,
    DesktopEntry,
    CustomShell { script: String },
    WorkspaceHook { context: PathBuf, sha256: Sha256Digest },
}

impl From<gripsack_ir::Action> for ActivationAction {
    fn from(action: gripsack_ir::Action) -> Self {
        match action {
            gripsack_ir::Action::Service { name, user } => Self::Service { name, user },
            gripsack_ir::Action::Fonts => Self::Fonts,
            gripsack_ir::Action::DesktopEntry => Self::DesktopEntry,
            gripsack_ir::Action::CustomShell { script } => Self::CustomShell { script },
        }
    }
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
    fn legacy_action_bytes_preserve_durable_identity() {
        for legacy in [
            gripsack_ir::Action::Fonts,
            gripsack_ir::Action::DesktopEntry,
            gripsack_ir::Action::Service { name: "demo.service".into(), user: false },
            gripsack_ir::Action::Service { name: "demo.service".into(), user: true },
            gripsack_ir::Action::CustomShell { script: "printf exact".into() },
        ] {
            let before = serde_json::to_vec(&legacy).unwrap();
            let stored = ActivationAction::from(legacy);
            assert_eq!(serde_json::to_vec(&stored).unwrap(), before);
            assert_eq!(serde_json::from_slice::<ActivationAction>(&before).unwrap(), stored);
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
