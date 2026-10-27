//! Historical verification contracts retained with published artifacts.
//!
//! These records preserve the pre-workspace serialization contract; they are
//! history data, not declarations accepted by the current authoring IR.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RetainedVerify {
    BinaryRuns {
        path: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
    },
    FileExists {
        path: String,
    },
    Shell {
        script: String,
    },
    FileDeployed {
        path: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn retained_verification_wire_is_unchanged() {
        for wire in [
            json!({"kind": "binary_runs", "path": "bin/tool"}),
            json!({"kind": "binary_runs", "path": "bin/tool", "args": ["--version"]}),
            json!({"kind": "file_exists", "path": "config/tool"}),
            json!({"kind": "shell", "script": "test -f config/tool"}),
            json!({"kind": "file_deployed", "path": "~/.config/tool"}),
        ] {
            let retained: RetainedVerify = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(serde_json::to_value(retained).unwrap(), wire);
        }
    }

    #[test]
    fn retained_binary_defaults_and_extension_admission_are_unchanged() {
        let retained: RetainedVerify = serde_json::from_value(json!({
            "kind": "binary_runs", "path": "bin/tool", "historical_extension": true
        }))
        .unwrap();
        assert_eq!(
            retained,
            RetainedVerify::BinaryRuns {
                path: "bin/tool".into(),
                args: vec![],
            }
        );
        assert_eq!(
            serde_json::to_value(retained).unwrap(),
            json!({"kind": "binary_runs", "path": "bin/tool"})
        );
    }
}
