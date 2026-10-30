//! Audit payload claims from the same receipts consumed by native execution.
use super::{
    PACKAGE_RECEIPT, PackageReceipt, RECEIPT_VERSION, RECIPE_RECEIPT, RecipeReceipt,
    read_receipt_readonly,
};
use crate::{
    ExecError,
    workspace::integrity::{Expectations, bind_digest, failure},
};
use std::path::Path;

pub(in crate::workspace) fn package(
    root: &Path,
    expected: &mut Expectations<'_>,
) -> Result<(), ExecError> {
    let receipt: PackageReceipt = read_receipt_readonly(expected.home, root, PACKAGE_RECEIPT)?
        .ok_or_else(|| failure("retained package object is missing"))?;
    if !receipt.supported_version() {
        return Err(failure(
            "retained package receipt version is unsupported; preserving its bytes",
        ));
    }
    bind_digest(root, "workspace-package", receipt.package.bytes())?;
    for (identity, path) in &receipt.runtime {
        expected.admit_reference(path)?;
        bind_digest(path, "workspace-package", identity.bytes())?;
    }
    for path in &receipt.producer_retention {
        expected.retain(path)?;
    }
    crate::workspace::sharing::readmit_roots(
        expected.home,
        receipt
            .producer_retention
            .iter()
            .map(std::path::PathBuf::as_path),
    )?;
    if let Some(conda) = &receipt.conda {
        gripsack_conda::virtuals::validate_frozen_environment(&conda.closure, None)
            .map_err(|error| failure(error.to_string()))?;
        let (key, prefix) =
            crate::workspace::conda::native_location("workspace", expected.home, &conda.closure)?;
        if conda.materialization != key
            || conda.final_prefix != prefix
            || receipt.producer_root != prefix
            || receipt.producer_payload != prefix
            || conda.platform != conda.closure.platform
        {
            return Err(failure(
                "retained native prefix differs from its package derivation",
            ));
        }
    }
    expected.tree(
        &receipt.producer_root,
        &receipt.producer_payload,
        receipt.tree,
    )?;
    for path in receipt.runtime.values() {
        expected.retain(path)?;
    }
    if let Some(conda) = receipt.conda {
        for package in &conda.closure.packages {
            expected.archive(&package.sha256)?;
        }
    }
    Ok(())
}

pub(in crate::workspace) fn recipe(
    root: &Path,
    expected: &mut Expectations<'_>,
) -> Result<(), ExecError> {
    let receipt: RecipeReceipt = read_receipt_readonly(expected.home, root, RECIPE_RECEIPT)?
        .ok_or_else(|| failure("retained recipe object is missing"))?;
    if receipt.version != RECEIPT_VERSION {
        return Err(failure(
            "retained recipe receipt version is unsupported; preserving its bytes",
        ));
    }
    bind_digest(root, "workspace-recipe", receipt.recipe.bytes())?;
    for path in &receipt.retention {
        expected.admit_reference(path)?;
    }
    expected.tree(root, &root.join("payload"), receipt.tree)?;
    for path in &receipt.retention {
        expected.retain(path)?;
    }
    Ok(())
}
