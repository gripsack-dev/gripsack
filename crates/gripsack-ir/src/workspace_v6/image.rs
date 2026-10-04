//! OCI consumers select retained runtime packages; they do not own production.
use super::{InstallPrefix, WorkspaceArg, WorkspacePlatform};
use crate::Span;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageOutput {
    pub name: String,
    pub span: Span,
    pub packages: Vec<String>,
    pub target: WorkspacePlatform,
    /// No base means scratch. An explicit base is a digest-pinned registry image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Explicit placements, including any transitive runtime dependency needing
    /// a fixed prefix. Unmapped relocatable packages use /opt/gripsack/<name>.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub destinations: BTreeMap<String, ImageDestination>,
    #[serde(default)]
    pub config: ImageRuntimeConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageDestination {
    pub path: InstallPrefix,
    #[serde(default)]
    pub owner: ImageOwner,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageOwner {
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageRuntimeConfig {
    /// Only literals and package commands are image-runtime arguments. Captured
    /// host inputs and production source/output bindings cannot escape here.
    #[serde(default)]
    pub entrypoint: Vec<WorkspaceArg>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default = "root_directory")]
    pub cwd: String,
    #[serde(default)]
    pub user: ImageOwner,
}
impl Default for ImageRuntimeConfig {
    fn default() -> Self {
        Self {
            entrypoint: Vec::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: root_directory(),
            user: ImageOwner::default(),
        }
    }
}
fn root_directory() -> String {
    "/".to_owned()
}
