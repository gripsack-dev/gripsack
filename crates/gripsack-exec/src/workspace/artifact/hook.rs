//! Hook snapshots bind existing package receipts, not serialized native loader
//! authority. Replay verifies their exact evidence and payload before admission.
use super::*;
use crate::workspace::sharing::RetainedInputSnapshot;
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
    if !receipt.supported_version()
        || receipt.package != package.identity
        || receipt.producer_root != package.producer.root
        || receipt.producer_payload != package.producer.payload
        || receipt.tree != package.producer.tree
        || receipt.commands != package.commands
        || receipt.target != package.target
        || receipt.layout != package.layout
        || receipt.conda != package.conda
        || receipt.host_runtime != package.host_runtime
        || receipt.archive_root != package.archive_root
        || receipt.selected_root != package.selected_root
        || receipt.producer_retention != portable_retention(&package.producer)
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

/// Package reuse belongs to one immutable activation receipt graph. Only import
/// evidence may be shared with another graph through the operation's admission.
pub(in crate::workspace) struct ReceiptRestoration<'a> {
    receipts: &'a BTreeMap<PackageDigest, Sha256Digest>,
    admission: &'a mut crate::workspace::sharing::ReadmissionScope,
    loaded: BTreeMap<PackageDigest, Arc<Package>>,
    evidence: BTreeMap<PackageDigest, (Arc<RetainedInputSnapshot>, Arc<RetainedInputSnapshot>)>,
    visiting: BTreeSet<PackageDigest>,
}

impl<'a> ReceiptRestoration<'a> {
    pub(in crate::workspace) fn new(
        receipts: &'a BTreeMap<PackageDigest, Sha256Digest>,
        admission: &'a mut crate::workspace::sharing::ReadmissionScope,
    ) -> Self {
        Self {
            receipts,
            admission,
            loaded: BTreeMap::new(),
            evidence: BTreeMap::new(),
            visiting: BTreeSet::new(),
        }
    }

    pub(in crate::workspace) fn restore(
        &mut self,
        identity: PackageDigest,
    ) -> Result<Arc<Package>, ExecError> {
        if let Some(package) = self.loaded.get(&identity) {
            // A cached parent includes its complete runtime graph. Recheck each
            // admitted input once, not once per path through a shared DAG.
            let mut pending = vec![package.as_ref()];
            let mut checked = BTreeSet::new();
            while let Some(package) = pending.pop() {
                if !checked.insert(package.identity) {
                    continue;
                }
                self.admission.check_input(&package.root)?;
                self.admission.check_input(&package.producer.root)?;
                self.admission
                    .readmit_roots(package.producer.retention.iter().map(PathBuf::as_path))?;
                pending.extend(package.runtime.iter().map(Arc::as_ref));
            }
            return Ok(Arc::clone(package));
        }
        if !self.visiting.insert(identity) {
            return Err(failure("retained hook package closure contains a cycle"));
        }
        let result = self.restore_uncached(identity);
        self.visiting.remove(&identity);
        if let Ok(package) = &result {
            self.loaded.insert(identity, Arc::clone(package));
        }
        result
    }

    fn restore_uncached(&mut self, identity: PackageDigest) -> Result<Arc<Package>, ExecError> {
        let expected = self
            .receipts
            .get(&identity)
            .ok_or_else(|| failure("retained hook package is absent from frozen closure"))?;
        let root = store::content_path(
            self.admission.home(),
            "workspace-package",
            &identity.to_string(),
        );
        let receipt_evidence = self.admission.pin_input(&root)?;
        let home = self.admission.home();
        let receipt: PackageReceipt = read_receipt_readonly(home, &root, PACKAGE_RECEIPT)?
            .ok_or_else(|| failure("retained hook package receipt is missing"))?;
        if !receipt.supported_version()
            || receipt.package != identity
            || Sha256Digest::of(&serde_json::to_vec(&receipt)?) != *expected
        {
            return Err(failure(
                "retained hook package receipt differs from activation snapshot",
            ));
        }
        store::paths::validate_store_root(home, &receipt.producer_root)?;
        let producer_evidence = self.admission.pin_input(&receipt.producer_root)?;
        let payload = receipt.producer_payload.canonicalize()?;
        if !payload.starts_with(&receipt.producer_root)
            || store::canonical_tree_hash(&payload)? != receipt.tree
        {
            return Err(failure(
                "retained hook package payload differs from its frozen receipt",
            ));
        }
        let selected_root =
            archive_root::select(receipt.archive_root.as_ref(), &receipt.layout, &payload)?;
        if selected_root != receipt.selected_root
            || (receipt.archive_root.is_some() && receipt.conda.is_some())
        {
            return Err(failure(
                "retained package archive root differs from its receipt",
            ));
        }
        let mut retention = receipt.producer_retention;
        for path in &retention {
            store::paths::validate_store_root(self.admission.home(), path)?;
        }
        let mut runtime = Vec::with_capacity(receipt.runtime.len());
        for (&dependency, path) in &receipt.runtime {
            let package = self.restore(dependency)?;
            if &package.root != path {
                return Err(failure(
                    "retained hook runtime root differs from its receipt",
                ));
            }
            runtime.push(package);
        }
        self.admission
            .readmit_roots(retention.iter().map(PathBuf::as_path))?;
        self.admission.check_input(&root)?;
        self.admission.check_input(&receipt.producer_root)?;
        self.evidence
            .insert(identity, (receipt_evidence, producer_evidence));
        retention.insert(receipt.producer_root.clone());
        Ok(Arc::new(Package {
            identity,
            root,
            selected_payload: selected_root.as_deref().map(|name| payload.join(name)),
            producer: Arc::new(Artifact {
                root: receipt.producer_root,
                payload,
                tree: receipt.tree,
                retention,
            }),
            archive_root: receipt.archive_root,
            selected_root,
            commands: receipt.commands,
            runtime,
            target: receipt.target,
            layout: receipt.layout,
            conda: receipt.conda,
            host_runtime: receipt.host_runtime,
        }))
    }

    pub(in crate::workspace) fn into_packages(self) -> BTreeMap<PackageDigest, Arc<Package>> {
        self.loaded
    }
}

/// A legacy host-runtime launcher authenticates its retained receipt graph.
/// Explicit profile launchers instead restore their versioned activation graph.
pub(in crate::workspace) fn restore_launcher(
    home: &Path,
    identity: PackageDigest,
    expected: Sha256Digest,
) -> Result<Arc<Package>, ExecError> {
    let root = store::content_path(home, "workspace-package", &identity.to_string());
    let receipt: PackageReceipt = read_receipt_readonly(home, &root, PACKAGE_RECEIPT)?
        .ok_or_else(|| failure("retained launcher package receipt is missing"))?;
    if !receipt.supported_version()
        || receipt.package != identity
        || Sha256Digest::of(&serde_json::to_vec(&receipt)?) != expected
    {
        return Err(failure(
            "retained launcher package receipt identity differs",
        ));
    }
    let mut receipts = receipt.runtime_receipts;
    if receipts.insert(identity, expected).is_some() {
        return Err(failure("retained launcher receipt graph contains itself"));
    }
    let mut admission = crate::workspace::sharing::ReadmissionScope::new(
        home,
        gripsack_fetch::FetchLimits::default(),
    );
    let package = ReceiptRestoration::new(&receipts, &mut admission).restore(identity)?;
    if !package.host_dependent() && !package.portable() {
        return Err(failure(
            "retained launcher package has no host runtime policy",
        ));
    }
    Ok(package)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::sharing::ReadmissionScope;
    use gripsack_fetch::FetchLimits;

    fn receipt(home: &Path) -> PackageReceipt {
        receipt_at(home, 1, 2)
    }

    fn identity(value: usize) -> PackageDigest {
        PackageDigest::parse(&Sha256Digest::of(&value.to_le_bytes()).to_string()).unwrap()
    }

    fn receipt_at(home: &Path, package: usize, producer: usize) -> PackageReceipt {
        let producer =
            store::content_path(home, "workspace-source", &identity(producer).to_string());
        std::fs::create_dir_all(&producer).unwrap();
        std::fs::write(producer.join("data"), b"frozen").unwrap();
        PackageReceipt {
            version: 1,
            package: identity(package),
            producer_root: producer.clone(),
            producer_payload: producer.clone(),
            tree: store::canonical_tree_hash(&producer).unwrap(),
            commands: BTreeMap::new(),
            runtime: BTreeMap::new(),
            target: serde_json::from_value(serde_json::json!({
                "os": "linux", "arch": "x86_64", "abi": "gnu"
            }))
            .unwrap(),
            layout: CatalogPackageLayout::Relocatable,
            conda: None,
            host_runtime: None,
            runtime_receipts: BTreeMap::new(),
            archive_root: None,
            selected_root: None,
            producer_retention: BTreeSet::new(),
        }
    }

    fn persist(home: &Path, receipt: &PackageReceipt) -> Sha256Digest {
        let root = store::content_path(home, "workspace-package", &receipt.package.to_string());
        std::fs::create_dir_all(&root).unwrap();
        let bytes = serde_json::to_vec(receipt).unwrap();
        std::fs::write(root.join(PACKAGE_RECEIPT), &bytes).unwrap();
        Sha256Digest::of(&bytes)
    }

    #[test]
    fn restoration_cache_is_bound_to_one_exact_receipt_view() {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path();
        let receipt = receipt(home);
        let expected = persist(home, &receipt);
        let view = BTreeMap::from([(receipt.package, expected)]);
        let mut admission = ReadmissionScope::new(home, FetchLimits::default());
        {
            let mut restoration = ReceiptRestoration::new(&view, &mut admission);
            let first = restoration.restore(receipt.package).unwrap();
            let repeated = restoration.restore(receipt.package).unwrap();
            assert!(Arc::ptr_eq(&first, &repeated));
            assert_eq!(restoration.into_packages().len(), 1);
        }
        let different = BTreeMap::from([(receipt.package, Sha256Digest::of(b"another view"))]);
        assert!(
            ReceiptRestoration::new(&different, &mut admission)
                .restore(receipt.package)
                .is_err()
        );
        assert!(
            ReceiptRestoration::new(&BTreeMap::new(), &mut admission)
                .restore(receipt.package)
                .is_err()
        );
    }

    #[test]
    fn shared_import_scope_does_not_reuse_a_packages_retention_view() {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path();
        let mut receipt = receipt(home);
        let old_view = BTreeMap::from([(receipt.package, persist(home, &receipt))]);
        let mut admission = ReadmissionScope::new(home, FetchLimits::default());
        let old = ReceiptRestoration::new(&old_view, &mut admission)
            .restore(receipt.package)
            .unwrap();
        let retained = store::content_path(home, "workspace-source", &"3".repeat(64));
        std::fs::create_dir_all(&retained).unwrap();
        receipt.version = PACKAGE_RECEIPT_PORTABLE;
        receipt.producer_retention.insert(retained.clone());
        let new_view = BTreeMap::from([(receipt.package, persist(home, &receipt))]);
        assert!(
            ReceiptRestoration::new(&old_view, &mut admission)
                .restore(receipt.package)
                .is_err()
        );
        assert!(
            ReceiptRestoration::new(&new_view, &mut admission)
                .restore(receipt.package)
                .is_err()
        );
        let mut admission = ReadmissionScope::new(home, FetchLimits::default());
        let new = ReceiptRestoration::new(&new_view, &mut admission)
            .restore(receipt.package)
            .unwrap();
        assert!(!Arc::ptr_eq(&old, &new));
        assert!(!old.producer.retention.contains(&retained));
        assert!(new.producer.retention.contains(&retained));
        std::fs::write(receipt.producer_payload.join("data"), b"tampered").unwrap();
        assert!(
            ReceiptRestoration::new(&new_view, &mut admission)
                .restore(receipt.package)
                .is_err()
        );
    }

    #[test]
    fn cached_restoration_rejects_changed_or_missing_input() {
        for change_payload in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let home = temporary.path();
            let receipt = receipt(home);
            let view = BTreeMap::from([(receipt.package, persist(home, &receipt))]);
            let mut admission = ReadmissionScope::new(home, FetchLimits::default());
            let mut restoration = ReceiptRestoration::new(&view, &mut admission);
            restoration.restore(receipt.package).unwrap();
            if change_payload {
                std::fs::write(receipt.producer_payload.join("data"), b"edited").unwrap();
            } else {
                let root =
                    store::content_path(home, "workspace-package", &receipt.package.to_string());
                std::fs::remove_file(root.join(PACKAGE_RECEIPT)).unwrap();
            }
            assert!(restoration.restore(receipt.package).is_err());
        }
    }

    #[test]
    fn cached_parent_rechecks_its_runtime_receipt_graph() {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path();
        let child = receipt(home);
        let mut parent = receipt(home);
        parent.package = serde_json::from_str(&format!("\"{}\"", "4".repeat(64))).unwrap();
        let child_root = store::content_path(home, "workspace-package", &child.package.to_string());
        parent.runtime.insert(child.package, child_root.clone());
        let view = BTreeMap::from([
            (child.package, persist(home, &child)),
            (parent.package, persist(home, &parent)),
        ]);
        let mut admission = ReadmissionScope::new(home, FetchLimits::default());
        let mut restoration = ReceiptRestoration::new(&view, &mut admission);
        let restored = restoration.restore(parent.package).unwrap();
        assert_eq!(restored.runtime.len(), 1);
        assert!(Arc::ptr_eq(
            &restored.runtime[0],
            &restoration.restore(child.package).unwrap()
        ));
        std::fs::remove_file(child_root.join(PACKAGE_RECEIPT)).unwrap();
        assert!(restoration.restore(parent.package).is_err());
    }

    #[test]
    fn failed_restoration_is_not_cached_or_left_visiting() {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path();
        let receipt = receipt(home);
        let expected = Sha256Digest::of(&serde_json::to_vec(&receipt).unwrap());
        let view = BTreeMap::from([(receipt.package, expected)]);
        let mut admission = ReadmissionScope::new(home, FetchLimits::default());
        let mut restoration = ReceiptRestoration::new(&view, &mut admission);
        assert!(restoration.restore(receipt.package).is_err());
        assert!(restoration.loaded.is_empty());
        assert!(restoration.visiting.is_empty());
        assert_eq!(persist(home, &receipt), expected);
        assert!(restoration.restore(receipt.package).is_err());
        drop(restoration);
        let mut fresh = ReadmissionScope::new(home, FetchLimits::default());
        assert!(
            ReceiptRestoration::new(&view, &mut fresh)
                .restore(receipt.package)
                .is_ok()
        );
    }

    #[test]
    fn scc_base_and_view_receipts_share_one_pin_per_distinct_root() {
        const MEMBERS: usize = 256;
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path();
        let mut receipts = BTreeMap::new();
        for member in 1..=MEMBERS {
            let mut receipt = receipt_at(home, member, member);
            receipts.insert(receipt.package, persist(home, &receipt));
            // SCC publication separates immutable bases from runtime views;
            // each view keeps its own receipt but shares its base's producer.
            receipt.package = identity(MEMBERS + member);
            let next = identity(member % MEMBERS + 1);
            receipt.runtime.insert(
                next,
                store::content_path(home, "workspace-package", &next.to_string()),
            );
            receipts.insert(receipt.package, persist(home, &receipt));
        }
        let mut admission = ReadmissionScope::new(home, FetchLimits::default());
        // Model producer pins already retained by imported-root admission.
        let _producer_pins: Vec<_> = (1..=MEMBERS)
            .map(|member| {
                let root =
                    store::content_path(home, "workspace-source", &identity(member).to_string());
                admission.pin_input(&root).unwrap()
            })
            .collect();
        {
            let mut restoration = ReceiptRestoration::new(&receipts, &mut admission);
            for member in 1..=MEMBERS {
                let view = restoration.restore(identity(MEMBERS + member)).unwrap();
                assert_eq!(view.runtime[0].identity, identity(member % MEMBERS + 1));
            }
            assert_eq!(restoration.loaded.len(), 2 * MEMBERS);
        }
        assert_eq!(admission.pinned_inputs(), 3 * MEMBERS);
        // Separate exact child views must reuse pins, not package graph caches.
        for member in 1..=MEMBERS {
            let package = identity(MEMBERS + member);
            let dependency = identity(member % MEMBERS + 1);
            let view = BTreeMap::from([
                (package, receipts[&package]),
                (dependency, receipts[&dependency]),
            ]);
            let mut restoration = ReceiptRestoration::new(&view, &mut admission);
            restoration.restore(package).unwrap();
            assert_eq!(restoration.loaded.len(), 2);
        }
        assert_eq!(admission.pinned_inputs(), 3 * MEMBERS);
    }
}
