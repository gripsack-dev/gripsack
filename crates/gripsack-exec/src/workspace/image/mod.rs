//! Read-only admission of historical OCI image receipts.
//! Stored archive bytes remain verifiable; no image authoring backend exists.
pub(super) mod materialize;

use super::artifact;
use crate::ExecError;
use gripsack_process::Sha256Digest;
use serde::Deserialize;
use std::path::Path;
const RECEIPT: &str = "image.json";
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageReceipt {
    version: u32,
    identity: Sha256Digest,
    archive_tree: gripsack_store::hash::PayloadHash,
    #[serde(rename = "manifest", deserialize_with = "manifest_digest")]
    _manifest: Sha256Digest,
    #[serde(rename = "external_runtime_requirements")]
    _external_runtime_requirements: Vec<String>,
}

pub(super) fn integrity(
    root: &Path,
    expected: &mut crate::workspace::integrity::Expectations<'_>,
) -> Result<(), ExecError> {
    use crate::workspace::integrity::{bind_digest, failure};
    let receipt: ImageReceipt = artifact::read_receipt_readonly(expected.home, root, RECEIPT)?
        .ok_or_else(|| failure("retained image object is missing"))?;
    if receipt.version != 2 {
        return Err(failure(
            "retained image receipt version is unsupported; preserving its bytes",
        ));
    }
    bind_digest(root, "workspace-image", receipt.identity.as_bytes())?;
    expected.tree(root, &root.join("payload"), receipt.archive_tree)
}

fn manifest_digest<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<Sha256Digest, D::Error> {
    let value = String::deserialize(decoder)?;
    let raw = value
        .strip_prefix("sha256:")
        .ok_or_else(|| serde::de::Error::custom("OCI digest must use sha256"))?;
    Sha256Digest::parse(raw).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn historical_image_receipts_require_prefixed_manifest_digests() {
        let digest = "a".repeat(64);
        let receipt = json!({
            "version": 2,
            "identity": digest,
            "archive_tree": digest,
            "manifest": format!("sha256:{digest}"),
            "external_runtime_requirements": []
        });
        assert!(serde_json::from_value::<ImageReceipt>(receipt.clone()).is_ok());
        for manifest in [
            digest.clone(),
            format!("sha512:{digest}"),
            "sha256:bad".into(),
        ] {
            let mut corrupt = receipt.clone();
            corrupt["manifest"] = json!(manifest);
            assert!(serde_json::from_value::<ImageReceipt>(corrupt).is_err());
        }
    }
}
