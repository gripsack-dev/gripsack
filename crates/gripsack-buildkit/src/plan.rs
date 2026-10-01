//! The bounded backend projection of an admitted workspace production graph.
//! This is not a second authoring IR: host tasks, destinations, credentials and
//! worker lifecycle identities cannot be represented in its operations.
mod admission;
mod exporter;
pub use exporter::{ExporterPlan, ImageConfig, MAX_IMAGE_CONFIG_BYTES};

use crate::identity::SnapshotDigest;
use serde::{Deserialize, Serialize};

pub const MAX_PLAN_NODES: usize = 256;
pub const MAX_DEFINITION_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct NodeIndex(u32);
impl NodeIndex {
    pub fn new(index: usize) -> Result<Self, PlanError> {
        if index >= MAX_PLAN_NODES {
            return Err(PlanError::NodeLimit);
        }
        Ok(Self(index as u32))
    }
    pub fn index(self) -> usize {
        self.0 as usize
    }
}
impl TryFrom<u32> for NodeIndex {
    type Error = PlanError;
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value as usize)
    }
}
impl From<NodeIndex> for u32 {
    fn from(value: NodeIndex) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinuxOs {
    Linux,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Architecture {
    Amd64,
    Arm64,
}
impl Architecture {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Platform {
    pub os: LinuxOs,
    pub architecture: Architecture,
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    pub source: Option<NodeIndex>,
    pub destination: String,
    pub readonly: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    Image {
        reference: String,
    },
    Local {
        name: String,
        digest: SnapshotDigest,
    },
    File {
        input: Option<NodeIndex>,
        path: String,
        #[serde(with = "crate::protocol::bytes")]
        data: Vec<u8>,
        mode: u32,
    },
    Directory {
        input: Option<NodeIndex>,
        path: String,
        mode: u32,
    },
    Copy {
        input: Option<NodeIndex>,
        source: NodeIndex,
        source_path: String,
        destination: String,
        contents: bool,
    },
    /// Image placement binds UID/GID explicitly; portable artifact copies do not.
    Install {
        input: Option<NodeIndex>,
        source: NodeIndex,
        source_path: String,
        destination: String,
        contents: bool,
        uid: u32,
        gid: u32,
    },
    Process {
        root: NodeIndex,
        argv: Vec<String>,
        env: Vec<String>,
        cwd: String,
        mounts: Vec<Mount>,
        output: String,
    },
}
impl Node {
    pub(crate) fn visit_inputs(&self, mut visit: impl FnMut(NodeIndex)) {
        match self {
            Self::Image { .. } | Self::Local { .. } => {}
            Self::File { input, .. } | Self::Directory { input, .. } => {
                if let Some(index) = input {
                    visit(*index);
                }
            }
            Self::Copy { input, source, .. } | Self::Install { input, source, .. } => {
                if let Some(index) = input {
                    visit(*index);
                }
                visit(*source);
            }
            Self::Process { root, mounts, .. } => {
                visit(*root);
                for mount in mounts {
                    if let Some(index) = mount.source {
                        visit(index);
                    }
                }
            }
        }
    }
}

/// Decoded or caller-constructed data carries no execution authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildPlan {
    pub platform: Platform,
    pub nodes: Vec<Node>,
    pub root: NodeIndex,
    pub exporter: ExporterPlan,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlanError {
    #[error("a build plan must contain 1..={MAX_PLAN_NODES} nodes")]
    NodeLimit,
    #[error("node {node} has a missing, forward or cyclic producer edge")]
    Edge { node: usize },
    #[error("node {node} is unreachable from the selected output")]
    Unreachable { node: usize },
    #[error("invalid build node {node}: {reason}")]
    Operation { node: usize, reason: &'static str },
    #[error("invalid exporter configuration: {0}")]
    Exporter(&'static str),
}

/// Admission establishes topology, explicit source authority and process policy.
/// Only the independent byte checker can turn this into a CheckedDefinition.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct ValidatedBuildPlan(BuildPlan);
impl ValidatedBuildPlan {
    pub fn admit(plan: BuildPlan) -> Result<Self, PlanError> {
        admission::validate(&plan)?;
        Ok(Self(plan))
    }
    pub fn plan(&self) -> &BuildPlan {
        &self.0
    }
}
