//! Read-only integrity admission of retained Conda image prefixes.
use crate::{
    ExecError,
    workspace::{
        artifact,
        conda::{self, CondaRuntimeReceipt},
    },
};
use serde::Deserialize;

const NAMESPACE: &str = "workspace-conda-image";
const RECEIPT: &str = "conda-image.json";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u32,
    tree: gripsack_store::hash::PayloadHash,
    runtime: CondaRuntimeReceipt,
}

pub(in crate::workspace) fn integrity(
    root: &std::path::Path,
    expected: &mut crate::workspace::integrity::Expectations<'_>,
) -> Result<(), ExecError> {
    use crate::workspace::integrity::{bind_identity, failure};
    let receipt: Receipt = artifact::read_receipt_readonly(expected.home, root, RECEIPT)?
        .ok_or_else(|| failure("retained image-prefix object is missing"))?;
    if receipt.version != 1 {
        return Err(failure(
            "retained image-prefix receipt version is unsupported; preserving its bytes",
        ));
    }
    let prefix = receipt
        .runtime
        .final_prefix
        .to_str()
        .ok_or_else(|| failure("retained image prefix is not UTF-8"))?;
    let key = conda::image_key(&receipt.runtime.closure, prefix);
    bind_identity(root, NAMESPACE, &key)?;
    if receipt.runtime.materialization != key
        || receipt.runtime.platform != receipt.runtime.closure.platform
    {
        return Err(failure(
            "retained image-prefix derivation differs from its receipt",
        ));
    }
    for package in &receipt.runtime.closure.packages {
        expected.archive(&package.sha256)?;
    }
    expected.tree(root, &root.join("payload"), receipt.tree)
}
