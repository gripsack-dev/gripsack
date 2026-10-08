//! Hook snapshots bind existing package receipts, not serialized native loader
//! authority. Replay verifies their exact evidence and payload before admission.
use super::*;
use gripsack_process::Sha256Digest;

pub(in crate::workspace) fn capture(
    home: &Path,
    package: &Package,
    receipts: &mut BTreeMap<PackageDigest, Sha256Digest>,
) -> Result<(), ExecError> {
    if receipts.contains_key(&package.identity) {
        return Ok(());
    }
    let receipt: PackageReceipt = read_receipt_readonly(home, &package.root, PACKAGE_RECEIPT)?
        .ok_or_else(|| failure("hook package receipt is missing"))?;
    if receipt.version != RECEIPT_VERSION
        || receipt.package != package.identity
        || receipt.producer_root != package.producer.root
        || receipt.producer_payload != package.producer.payload
        || receipt.tree != package.producer.tree
        || receipt.commands != package.commands
        || receipt.target != package.target
        || receipt.layout != package.layout
        || receipt.conda != package.conda
        || receipt.runtime.len() != package.runtime.len()
        || package
            .runtime
            .iter()
            .any(|runtime| receipt.runtime.get(&runtime.identity) != Some(&runtime.root))
    {
        return Err(failure("hook package receipt identity differs"));
    }
    receipts.insert(
        package.identity,
        Sha256Digest::of(&serde_json::to_vec(&receipt)?),
    );
    for runtime in &package.runtime {
        capture(home, runtime, receipts)?;
    }
    Ok(())
}

pub(in crate::workspace) fn restore(
    home: &Path,
    identity: PackageDigest,
    receipts: &BTreeMap<PackageDigest, Sha256Digest>,
    loaded: &mut BTreeMap<PackageDigest, Arc<Package>>,
    visiting: &mut BTreeSet<PackageDigest>,
) -> Result<Arc<Package>, ExecError> {
    if let Some(package) = loaded.get(&identity) {
        return Ok(Arc::clone(package));
    }
    if !visiting.insert(identity) {
        return Err(failure("retained hook package closure contains a cycle"));
    }
    let expected = receipts
        .get(&identity)
        .ok_or_else(|| failure("retained hook package is absent from frozen closure"))?;
    let root = store::content_path(home, "workspace-package", &identity.to_string());
    let receipt: PackageReceipt = read_receipt_readonly(home, &root, PACKAGE_RECEIPT)?
        .ok_or_else(|| failure("retained hook package receipt is missing"))?;
    if receipt.version != RECEIPT_VERSION
        || receipt.package != identity
        || Sha256Digest::of(&serde_json::to_vec(&receipt)?) != *expected
    {
        return Err(failure(
            "retained hook package receipt differs from activation snapshot",
        ));
    }
    store::paths::validate_store_root(home, &receipt.producer_root)?;
    let payload = receipt.producer_payload.canonicalize()?;
    if !payload.starts_with(&receipt.producer_root)
        || store::canonical_tree_hash(&payload)? != receipt.tree
    {
        return Err(failure(
            "retained hook package payload differs from its frozen receipt",
        ));
    }
    let mut runtime = Vec::with_capacity(receipt.runtime.len());
    for (&dependency, path) in &receipt.runtime {
        let package = restore(home, dependency, receipts, loaded, visiting)?;
        if &package.root != path {
            return Err(failure(
                "retained hook runtime root differs from its receipt",
            ));
        }
        runtime.push(package);
    }
    let package = Arc::new(Package {
        identity,
        root,
        producer: Arc::new(Artifact {
            root: receipt.producer_root.clone(),
            payload,
            tree: receipt.tree,
            retention: BTreeSet::from([receipt.producer_root]),
        }),
        commands: receipt.commands,
        runtime,
        target: receipt.target,
        layout: receipt.layout,
        conda: receipt.conda,
    });
    visiting.remove(&identity);
    loaded.insert(identity, Arc::clone(&package));
    Ok(package)
}
