//! Strict current persisted-action seam. Legacy declarations retain their
//! historical IR reader; migration emits this complete versioned shape.
use gripsack_ir::Action;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub(super) fn serialize<S: Serializer>(action: &Action, serializer: S) -> Result<S::Ok, S::Error> {
    action.serialize(serializer)
}

pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Action, D::Error> {
    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    enum Wire {
        Service { name: String, user: bool },
        Fonts,
        DesktopEntry,
        CustomShell { script: String },
    }
    Ok(match Wire::deserialize(deserializer)? {
        Wire::Service { name, user } => Action::Service { name, user },
        Wire::Fonts => Action::Fonts,
        Wire::DesktopEntry => Action::DesktopEntry,
        Wire::CustomShell { script } => Action::CustomShell { script },
    })
}
