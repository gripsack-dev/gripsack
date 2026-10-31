//! Exact independent-manifest DAGs. Local member cycles remain manifest-local.
use super::{
    format::{self, BlobDescriptor, PortableManifest},
    receipt,
    runtime::failure,
};
use crate::ExecError;
use gripsack_fetch::FetchLimits;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const MAX_CLOSURE_MANIFESTS: usize = 128;
pub const MAX_CLOSURE_DEPTH: usize = 32;
pub const MAX_CLOSURE_METADATA_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CLOSURE_EXPANDED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub struct ExternalManifest {
    pub manifest: Vec<u8>,
    pub envelope: Vec<u8>,
}

pub struct ClosureBundle {
    pub manifest: Vec<u8>,
    pub envelope: Vec<u8>,
    pub external_manifests: BTreeMap<String, ExternalManifest>,
    pub blobs: BTreeMap<String, Vec<u8>>,
}

pub(super) struct Closure {
    pub root: String,
    pub objects: BTreeMap<String, ExternalManifest>,
    pub manifests: BTreeMap<String, PortableManifest>,
    pub blobs: BTreeMap<String, BlobDescriptor>,
}

fn records(directory: &gripsack_fs::Dir) -> Result<ExternalManifest, ExecError> {
    Ok(ExternalManifest {
        manifest: receipt::read(
            directory,
            Path::new("manifest.json"),
            format::MAX_MANIFEST_BYTES as u64,
        )?,
        envelope: receipt::read(
            directory,
            Path::new("envelope.json"),
            super::trust::MAX_ENVELOPE_BYTES as u64,
        )?,
    })
}

fn account_metadata(total: &mut usize, object: &ExternalManifest) -> Result<(), ExecError> {
    if object.envelope.len() > super::trust::MAX_ENVELOPE_BYTES {
        return Err(failure("closure envelope exceeds byte limit"));
    }
    *total = total
        .checked_add(object.manifest.len())
        .and_then(|n| n.checked_add(object.envelope.len()))
        .ok_or_else(|| failure("closure metadata byte count overflow"))?;
    if *total > MAX_CLOSURE_METADATA_BYTES {
        return Err(failure("closure metadata exceeds byte limit"));
    }
    Ok(())
}

impl Closure {
    pub fn read(directory: &gripsack_fs::Dir, limits: FetchLimits) -> Result<Self, ExecError> {
        let root = records(directory)?;
        Self::resolve(root, limits, |identity| {
            let path = format!("manifests/{identity}");
            let child =
                gripsack_fs::open_dir_nofollow(directory, Path::new(&path)).map_err(|error| {
                    failure(format!("missing external manifest {identity}: {error}"))
                })?;
            records(&child)
                .map_err(|error| failure(format!("external manifest {identity}: {error}")))
        })
    }

    fn resolve(
        root: ExternalManifest,
        limits: FetchLimits,
        mut read: impl FnMut(&str) -> Result<ExternalManifest, ExecError>,
    ) -> Result<Self, ExecError> {
        let mut metadata = 0usize;
        account_metadata(&mut metadata, &root)?;
        let root_manifest = PortableManifest::decode(&root.manifest)?;
        let identity = root_manifest.digest()?;
        let mut closure = Self {
            root: identity.clone(),
            objects: BTreeMap::from([(identity.clone(), root)]),
            manifests: BTreeMap::from([(identity.clone(), root_manifest)]),
            blobs: BTreeMap::new(),
        };
        let mut pending = vec![identity];
        while let Some(identity) = pending.pop() {
            let references: Vec<_> = closure.manifests[&identity]
                .components
                .iter()
                .filter_map(|component| component.external.clone())
                .collect();
            for reference in references {
                if !closure.manifests.contains_key(&reference.manifest) {
                    if closure.manifests.len() >= MAX_CLOSURE_MANIFESTS {
                        return Err(failure("external manifest count exceeds limit"));
                    }
                    let object = read(&reference.manifest)?;
                    account_metadata(&mut metadata, &object)?;
                    let manifest = PortableManifest::decode(&object.manifest)?;
                    if manifest.digest()? != reference.manifest {
                        return Err(failure(
                            "external manifest digest differs from exact reference",
                        ));
                    }
                    pending.push(reference.manifest.clone());
                    closure.objects.insert(reference.manifest.clone(), object);
                    closure
                        .manifests
                        .insert(reference.manifest.clone(), manifest);
                }
                let child = &closure.manifests[&reference.manifest];
                if child.namespace != reference.namespace || child.publisher != reference.publisher
                {
                    return Err(failure(
                        "external namespace/publisher differs from exact reference",
                    ));
                }
            }
        }
        closure.check_graph(limits)?;
        let mut total = metadata as u64;
        let mut components = 0usize;
        for manifest in closure.manifests.values() {
            components = components
                .checked_add(manifest.components.len())
                .ok_or_else(|| failure("closure component count overflow"))?;
            if components > format::MAX_COMPONENTS {
                return Err(failure("complete closure exceeds component count limit"));
            }
            for component in &manifest.components {
                for blob in component.payload.iter().chain(
                    component
                        .conda
                        .iter()
                        .flat_map(|record| record.descriptors()),
                ) {
                    if let Some(previous) = closure.blobs.get(&blob.sha256) {
                        if previous != blob {
                            return Err(failure(
                                "conflicting descriptors for the same closure blob",
                            ));
                        }
                    } else {
                        total = total
                            .checked_add(blob.size)
                            .ok_or_else(|| failure("closure byte count overflow"))?;
                        closure.blobs.insert(blob.sha256.clone(), blob.clone());
                    }
                }
            }
        }
        if total > limits.download_bytes.get().min(format::MAX_PAYLOAD_BYTES) {
            return Err(failure("complete closure exceeds acquisition byte limit"));
        }
        Ok(closure)
    }

    pub(super) fn check_graph(&self, limits: FetchLimits) -> Result<(), ExecError> {
        self.native_budget(limits).map(|_| ())
    }

    fn native_budget(
        &self,
        limits: FetchLimits,
    ) -> Result<crate::workspace::conda::AdmissionBudget, ExecError> {
        self.graph_depth(&self.root, &mut BTreeSet::new(), &mut BTreeMap::new())?;
        let mut budget = crate::workspace::conda::AdmissionBudget {
            bytes: limits.expanded_bytes.get().min(MAX_CLOSURE_EXPANDED_BYTES),
            entries: limits.archive_entries.get().min(format::MAX_ENTRIES),
        };
        self.visit(
            &self.root,
            0,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            false,
            &mut budget,
        )?;
        Ok(budget)
    }

    /// Admit all Conda originals before the first member helper or publication.
    /// Transport blobs are deduplicated separately. Conda prefixes and independent
    /// environment members are realized once; embedded native trees are charged
    /// for every actual occurrence, never treated as prefix copies.
    pub(super) fn preflight_conda(
        &self,
        home: &Path,
        blobs: Option<&Path>,
        limits: FetchLimits,
    ) -> Result<super::conda::PreparedConda, ExecError> {
        let mut budget = self.native_budget(limits)?;
        let mut prefixes = BTreeMap::new();
        for manifest in self.manifests.values() {
            for component in &manifest.components {
                if let Some(record) = &component.conda {
                    let (identity, _) = crate::workspace::conda::native_location(
                        &component.id,
                        home,
                        &record.lock,
                    )?;
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        prefixes.entry(identity)
                    {
                        entry.insert(super::conda::preflight(
                            &component.id,
                            home,
                            record,
                            blobs,
                            &mut budget,
                        )?);
                    }
                }
            }
        }
        Ok(prefixes)
    }

    fn visit<'a>(
        &'a self,
        identity: &'a str,
        depth: usize,
        active: &mut BTreeSet<&'a str>,
        independent: &mut BTreeSet<&'a str>,
        embedded: bool,
        budget: &mut crate::workspace::conda::AdmissionBudget,
    ) -> Result<(), ExecError> {
        if !active.insert(identity) {
            return Err(failure("external manifest graph contains a cycle"));
        }
        if depth > MAX_CLOSURE_DEPTH {
            return Err(failure("external manifest graph exceeds depth limit"));
        }
        if !embedded && !independent.insert(identity) {
            active.remove(identity);
            return Ok(());
        }
        let manifest = self
            .manifests
            .get(identity)
            .ok_or_else(|| failure("missing external manifest in closure"))?;
        budget.charge("portable closure", 0, 2)?;
        for component in &manifest.components {
            budget.charge("portable closure", 0, 1 + component.inventory.len())?;
            for entry in &component.inventory {
                if let format::EntryKind::File { size, .. } = &entry.kind {
                    budget.charge("portable closure", *size, 0)?;
                }
            }
            if let Some(reference) = &component.external {
                let embedded = !matches!(manifest.layout, format::RuntimeLayout::Environment {});
                self.visit(
                    &reference.manifest,
                    depth + 1,
                    active,
                    independent,
                    embedded,
                    budget,
                )?;
            }
        }
        active.remove(identity);
        Ok(())
    }

    fn graph_depth<'a>(
        &'a self,
        identity: &'a str,
        active: &mut BTreeSet<&'a str>,
        depths: &mut BTreeMap<&'a str, usize>,
    ) -> Result<usize, ExecError> {
        if let Some(depth) = depths.get(identity) {
            return Ok(*depth);
        }
        if !active.insert(identity) {
            return Err(failure("external manifest graph contains a cycle"));
        }
        let manifest = self
            .manifests
            .get(identity)
            .ok_or_else(|| failure("missing external manifest in closure"))?;
        let mut depth = 0;
        for reference in manifest
            .components
            .iter()
            .filter_map(|component| component.external.as_ref())
        {
            depth = depth.max(1 + self.graph_depth(&reference.manifest, active, depths)?);
            if depth > MAX_CLOSURE_DEPTH {
                return Err(failure("external manifest graph exceeds depth limit"));
            }
        }
        active.remove(identity);
        depths.insert(identity, depth);
        Ok(depth)
    }

    pub fn write_records(&self, destination: &Path) -> Result<(), ExecError> {
        for (identity, object) in &self.objects {
            let path = if identity == &self.root {
                destination.to_path_buf()
            } else {
                destination.join("manifests").join(identity)
            };
            std::fs::create_dir_all(&path)?;
            std::fs::write(path.join("manifest.json"), &object.manifest)?;
            std::fs::write(path.join("envelope.json"), &object.envelope)?;
        }
        Ok(())
    }
}

impl ClosureBundle {
    pub fn validate(&self) -> Result<(), ExecError> {
        self.validate_with_limits(FetchLimits::default())
            .map(|_| ())
    }

    fn validate_with_limits(&self, limits: FetchLimits) -> Result<Closure, ExecError> {
        let graph = Closure::resolve(
            ExternalManifest {
                manifest: self.manifest.clone(),
                envelope: self.envelope.clone(),
            },
            limits,
            |identity| {
                let value = self
                    .external_manifests
                    .get(identity)
                    .ok_or_else(|| failure(format!("missing external manifest {identity}")))?;
                Ok(ExternalManifest {
                    manifest: value.manifest.clone(),
                    envelope: value.envelope.clone(),
                })
            },
        )?;
        if graph.manifests.len() != self.external_manifests.len() + 1
            || graph.blobs.len() != self.blobs.len()
        {
            return Err(failure(
                "bundle contains unreferenced manifest or blob objects",
            ));
        }
        for (identity, descriptor) in &graph.blobs {
            let bytes = self
                .blobs
                .get(identity)
                .ok_or_else(|| failure(format!("missing closure blob {identity}")))?;
            if bytes.len() as u64 != descriptor.size
                || gripsack_store::hash::hex_sha256(bytes) != *identity
            {
                return Err(failure("closure blob differs from signed descriptor"));
            }
        }
        Ok(graph)
    }

    pub fn read(path: &Path, limits: FetchLimits) -> Result<Self, ExecError> {
        let directory = gripsack_fs::open(path)?;
        let mut graph = Closure::read(&directory, limits)?;
        let source = gripsack_fs::open_dir_nofollow(&directory, Path::new("blobs"))?;
        let mut blobs = BTreeMap::new();
        for (identity, descriptor) in &graph.blobs {
            let bytes = receipt::read(&source, Path::new(identity), descriptor.size)?;
            if bytes.len() as u64 != descriptor.size
                || gripsack_store::hash::hex_sha256(&bytes) != *identity
            {
                return Err(failure("closure blob differs from signed descriptor"));
            }
            blobs.insert(identity.clone(), bytes);
        }
        let root = graph
            .objects
            .remove(&graph.root)
            .ok_or_else(|| failure("closure root missing"))?;
        Ok(Self {
            manifest: root.manifest,
            envelope: root.envelope,
            external_manifests: graph.objects,
            blobs,
        })
    }

    /// Writes into an attempt-owned destination. Publication belongs to the caller.
    pub fn write(&self, path: &Path) -> Result<(), ExecError> {
        let graph = self.validate_with_limits(FetchLimits::default())?;
        std::fs::create_dir_all(path.join("blobs"))?;
        graph.write_records(path)?;
        for (identity, bytes) in &self.blobs {
            std::fs::write(path.join("blobs").join(identity), bytes)?;
        }
        Ok(())
    }
}

/// Structural offline validation with streaming blob integrity checks; no trust
/// decision and no materialization or exposure effects.
pub fn validate_directory(path: &Path) -> Result<(), ExecError> {
    let directory = gripsack_fs::open(path)?;
    let graph = Closure::read(&directory, FetchLimits::default())?;
    let blobs = gripsack_fs::open_dir_nofollow(&directory, Path::new("blobs"))?;
    for descriptor in graph.blobs.values() {
        receipt::blob(&blobs, descriptor, None)?;
    }
    Ok(())
}
