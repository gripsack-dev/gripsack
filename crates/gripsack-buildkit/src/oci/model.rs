use super::BlobDigest;
use serde::Deserialize;
use std::collections::BTreeMap;

pub(super) const INDEX_MEDIA: &str = "application/vnd.oci.image.index.v1+json";
pub(super) const MANIFEST_MEDIA: &str = "application/vnd.oci.image.manifest.v1+json";
pub(super) const CONFIG_MEDIA: &str = "application/vnd.oci.image.config.v1+json";
pub(super) const LAYER_MEDIA: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
pub(super) const EPOCH: &str = "1970-01-01T00:00:00Z";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Layout {
    #[serde(rename = "imageLayoutVersion")]
    pub version: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Index {
    #[serde(rename = "schemaVersion")]
    pub version: u32,
    #[serde(default, rename = "mediaType")]
    pub media_type: Option<String>,
    pub manifests: Vec<Descriptor>,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Descriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: BlobDigest,
    pub size: u64,
    #[serde(default)]
    pub platform: Option<Platform>,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Platform {
    pub os: String,
    pub architecture: String,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default, rename = "os.version")]
    pub os_version: Option<String>,
    #[serde(default, rename = "os.features")]
    pub os_features: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    #[serde(rename = "schemaVersion")]
    pub version: u32,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub config: Descriptor,
    pub layers: Vec<Descriptor>,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Configuration {
    pub created: String,
    pub architecture: String,
    pub os: String,
    #[serde(default)]
    pub variant: Option<String>,
    pub config: Runtime,
    pub rootfs: RootFs,
    #[serde(default)]
    pub history: Vec<History>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub(super) struct Runtime {
    #[serde(default)]
    pub entrypoint: Vec<String>,
    #[serde(default, rename = "Cmd")]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    pub working_dir: String,
    pub user: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RootFs {
    #[serde(rename = "type")]
    pub kind: String,
    pub diff_ids: Vec<BlobDigest>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct History {
    pub created: String,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub empty_layer: bool,
    #[serde(default)]
    pub comment: String,
}
