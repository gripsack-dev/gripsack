//! Wire types shared by fetch and capability conversations.

use serde::Deserialize;

#[derive(serde::Serialize)]
pub(super) struct FetchRequest<'a> {
    pub op: &'static str,
    pub args: &'a serde_json::Value,
    pub dest_dir: std::borrow::Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<&'a serde_json::Value>,
}

#[derive(Deserialize)]
pub(super) struct PluginMessage {
    #[serde(rename = "type")]
    pub kind: String,
    pub diagnostic: Option<gripsack_ir::Diagnostic>,
    pub result: Option<PluginResult>,
}

#[derive(Deserialize, Default)]
pub(super) struct PluginResult {
    pub provenance: Option<serde_json::Value>,
    pub url: Option<String>,
    pub version: Option<String>,
    pub sha256: Option<String>,
    pub capabilities: Option<serde_json::Value>,
}
