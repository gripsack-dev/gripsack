//! Private, immutable per-profile review evidence. Generation build closures are
//! the only retention authority; imported review documents cannot name roots.
use super::{admission, model::*, normalize::subject};
use crate::LifecycleSession;
use gripsack_store::{DeploymentState, hash::PayloadHash};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const NAMESPACE: &str = "semantic-review";
const RECORD: &str = "snapshot.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    profile: SubjectRef,
    snapshot: SnapshotDocument,
}

pub(crate) fn is_root(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split_once('-'))
        .is_some_and(|(_, kind)| kind == NAMESPACE)
}

/// Full tree identity, rather than the ordinary abbreviated lookup key, binds
/// the retained evidence. The digest is private and never exported as a fact.
fn root_path(home: &Path, tree: &PayloadHash) -> PathBuf {
    home.join("store").join(format!("{tree}-{NAMESPACE}"))
}

pub(crate) fn publish_profile(
    session: &LifecycleSession,
    profile: &str,
    snapshot: SemanticSnapshot,
) -> Result<PathBuf, SnapshotError> {
    validate_binding(&snapshot, &subject("output", profile))?;
    let bytes = admission::encode_bounded(&Record {
        profile: subject("output", profile),
        snapshot: snapshot.into_document(),
    })?;
    let home = gripsack_fs::open(session.home())?;
    gripsack_fs::create_dir_all(&home, Path::new("store"))?;
    let store = gripsack_fs::open_dir_nofollow(&home, Path::new("store"))?;
    let tree = gripsack_store::hash::canonical_single_file_tree_hash(RECORD, &bytes);
    let root = root_path(session.home(), &tree);
    let name = Path::new(
        root.file_name()
            .ok_or(SnapshotError::Invalid("semantic root name"))?,
    );
    match store.symlink_metadata(name) {
        Ok(_) => {
            let retained = read_record(session.home(), &root)?;
            if admission::encode_bounded(&retained)? != bytes {
                return Err(SnapshotError::Invalid(
                    "semantic artifact identity collision",
                ));
            }
            let directory = gripsack_fs::open_dir_nofollow(&store, name)?;
            gripsack_fs::open_file_nofollow(&directory, Path::new(RECORD))?.sync_all()?;
            gripsack_fs::fsync_pinned_dir(&directory, name)?;
            gripsack_fs::fsync_pinned_dir(&store, Path::new("."))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            // Same-filesystem private staging; never an incomplete authority.
            let stage = tempfile::Builder::new()
                .prefix(".semantic-review-")
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir_in(session.home().join("store"))?;
            let file = stage.path().join(RECORD);
            {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o400)
                    .open(file)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
            let stage_name = Path::new(
                stage
                    .path()
                    .file_name()
                    .ok_or(SnapshotError::Invalid("semantic staging name"))?,
            );
            let stage_dir = gripsack_fs::open_dir_nofollow(&store, stage_name)?;
            gripsack_fs::fsync_pinned_dir(&stage_dir, Path::new("."))?;
            gripsack_fs::rename_noreplace(&store, stage_name, name)?;
            gripsack_fs::fsync_pinned_dir(&store, Path::new("."))?;
        }
        Err(error) => return Err(error.into()),
    }
    Ok(root)
}

fn validate_binding(
    snapshot: &SemanticSnapshot,
    profile: &SubjectRef,
) -> Result<(), SnapshotError> {
    if snapshot.document().origin != SnapshotOrigin::Recorded
        || snapshot.document().generation.is_some()
        || !snapshot
            .document()
            .nodes
            .iter()
            .any(|node| &node.id == profile && node.kind == SubjectKind::Profile)
    {
        return Err(SnapshotError::Invalid("retained snapshot profile binding"));
    }
    Ok(())
}

fn read_record(home: &Path, root: &Path) -> Result<Record, SnapshotError> {
    gripsack_store::paths::validate_store_root(home, root)?;
    if !is_root(root) {
        return Err(SnapshotError::Invalid("semantic artifact namespace"));
    }
    let home_dir = gripsack_fs::open(home)?;
    let store = gripsack_fs::open_dir_nofollow(&home_dir, Path::new("store"))?;
    let directory = gripsack_fs::open_dir_nofollow(
        &store,
        Path::new(
            root.file_name()
                .ok_or(SnapshotError::Invalid("semantic root name"))?,
        ),
    )?;
    {
        use gripsack_fs::cap_std::fs::MetadataExt;
        if directory.metadata(".")?.mode() & 0o077 != 0 {
            return Err(SnapshotError::Invalid(
                "semantic artifact directory is not private",
            ));
        }
    }
    let mut entries = directory.entries()?;
    let entry = entries
        .next()
        .ok_or(SnapshotError::Invalid("empty semantic artifact"))??;
    if entry.file_name() != RECORD || entries.next().is_some() {
        return Err(SnapshotError::Invalid(
            "semantic artifact must contain exactly its record",
        ));
    }
    let file = gripsack_fs::open_file_nofollow(&directory, Path::new(RECORD))?;
    let metadata = file.metadata()?;
    {
        use gripsack_fs::cap_std::fs::MetadataExt;
        if metadata.mode() & 0o377 != 0 || metadata.len() > MAX_SNAPSHOT_BYTES as u64 {
            return Err(SnapshotError::Invalid(
                "semantic artifact privacy or byte bound",
            ));
        }
    }
    let mut bytes = Vec::new();
    file.take((MAX_SNAPSHOT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(SnapshotError::Bound("bytes"));
    }
    // The complete exact tree is checked, including extra entries and mode.
    let tree = gripsack_store::hash::canonical_single_file_tree_hash(RECORD, &bytes);
    if root_path(home, &tree) != root {
        return Err(SnapshotError::Invalid("semantic artifact tree identity"));
    }
    let record: Record = serde_json::from_value(admission::parse_json(&bytes)?)?;
    let snapshot = SemanticSnapshot::admit(record.snapshot)?;
    validate_binding(&snapshot, &record.profile)?;
    Ok(Record {
        profile: record.profile,
        snapshot: snapshot.into_document(),
    })
}

/// Native audit reads exactly the same bounded artifact contract as history.
pub(crate) fn audit_root(home: &Path, root: &Path) -> Result<(), SnapshotError> {
    read_record(home, root).map(drop)
}

pub(crate) fn profile_root(deployment: &DeploymentState) -> Result<Option<&Path>, SnapshotError> {
    let mut roots = deployment.build_closure.iter().filter(|root| is_root(root));
    let root = roots.next();
    if roots.next().is_some() {
        return Err(SnapshotError::Invalid("ambiguous semantic profile roots"));
    }
    Ok(root.map(PathBuf::as_path))
}

pub(crate) fn read_profile(
    home: &Path,
    profile: &str,
    deployment: &DeploymentState,
) -> Result<Option<SemanticSnapshot>, SnapshotError> {
    let Some(root) = profile_root(deployment)? else {
        return Ok(None);
    };
    let record = read_record(home, root)?;
    if record.profile != subject("output", profile) {
        return Err(SnapshotError::Invalid(
            "semantic artifact belongs to another profile",
        ));
    }
    SemanticSnapshot::admit(record.snapshot).map(Some)
}

/// Reusing metadata does not claim fresh observations for an old generation.
/// Equality ignores only review provenance and observations, never unknowns or
/// semantic input/lock/receipt identities.
pub(crate) fn same_desired_state(left: &SemanticSnapshot, right: &SemanticSnapshot) -> bool {
    let meaningful =
        |fact: &&Fact| !matches!(fact.kind, ChangeKind::Provenance | ChangeKind::Observation);
    left.document().version == right.document().version
        && left.document().canonicalization_version == right.document().canonicalization_version
        && left.document().policy_version == right.document().policy_version
        && left.document().edges == right.document().edges
        && left.document().nodes.len() == right.document().nodes.len()
        && left
            .document()
            .nodes
            .iter()
            .zip(&right.document().nodes)
            .all(|(left, right)| {
                left.id == right.id
                    && left.kind == right.kind
                    && left
                        .facts
                        .iter()
                        .filter(meaningful)
                        .eq(right.facts.iter().filter(meaningful))
            })
}

/// Merge independent frozen scopes conservatively. Conflicting shared facts
/// become explicitly unknown; each original remains recoverable in its root.
pub(super) fn merge(document: &mut SnapshotDocument, snapshot: SemanticSnapshot, profile: &str) {
    let mut snapshot = snapshot.into_document();
    // Two individually valid frozen graphs may disagree about shared dependency
    // direction. Never invent a cyclic producer graph by flattening them.
    // Keep that second graph as an explicitly partial per-profile scope.
    separate_disputed_graph(document, &mut snapshot, profile);
    document.completeness.complete &= snapshot.completeness.complete;
    document.completeness.redacted |= snapshot.completeness.redacted;
    document.completeness.truncated |= snapshot.completeness.truncated;
    document
        .completeness
        .unknown_reasons
        .extend(snapshot.completeness.unknown_reasons);
    for incoming in snapshot.nodes {
        if let Some(node) = document
            .nodes
            .iter_mut()
            .find(|node| node.id == incoming.id)
        {
            let mut existing: BTreeMap<_, _> = node
                .facts
                .drain(..)
                .map(|fact| (fact.path.clone(), fact))
                .collect();
            let incoming_facts: BTreeMap<_, _> = incoming
                .facts
                .into_iter()
                .map(|fact| (fact.path.clone(), fact))
                .collect();
            let paths: BTreeSet<_> = existing
                .keys()
                .chain(incoming_facts.keys())
                .cloned()
                .collect();
            for path in paths {
                let left = existing.remove(&path);
                let right = incoming_facts.get(&path);
                let equal = left.as_ref() == right;
                let mut fact = left
                    .or_else(|| right.cloned())
                    .expect("union contains field");
                if !equal {
                    fact.knowledge = KnowledgeState::Unknown {
                        reason: UnknownReason::Unobserved,
                    };
                    fact.origin = EvidenceOrigin::Historical;
                    fact.observed_at = None;
                }
                node.facts.push(fact);
            }
            if node.kind != incoming.kind {
                node.kind = SubjectKind::Output;
            }
            for location in incoming.provenance {
                if !node.provenance.contains(&location) {
                    node.provenance.push(location);
                }
            }
        } else {
            document.nodes.push(incoming);
        }
    }
    document.edges.extend(snapshot.edges);
    document.edges.sort_unstable();
    document.edges.dedup();
}

fn separate_disputed_graph(
    document: &mut SnapshotDocument,
    snapshot: &mut SnapshotDocument,
    profile: &str,
) {
    let existing: BTreeSet<_> = document.nodes.iter().map(|node| node.id.clone()).collect();
    let incoming_edges = |edges: &[SnapshotEdge]| {
        let mut by_consumer: BTreeMap<SubjectRef, BTreeSet<(SubjectRef, EdgeKind)>> =
            BTreeMap::new();
        for edge in edges {
            by_consumer
                .entry(edge.consumer.clone())
                .or_default()
                .insert((edge.dependency.clone(), edge.kind));
        }
        by_consumer
    };
    let old = incoming_edges(&document.edges);
    let new = incoming_edges(&snapshot.edges);
    if !snapshot
        .nodes
        .iter()
        .any(|node| existing.contains(&node.id) && old.get(&node.id) != new.get(&node.id))
    {
        return;
    }
    let selected = subject("output", profile);
    let remapped: BTreeMap<_, _> = snapshot
        .nodes
        .iter()
        .filter(|node| node.id != selected)
        .map(|node| {
            (
                node.id.clone(),
                SubjectRef(format!(
                    "retained/{}/{}",
                    super::normalize::escaped(profile),
                    node.id.0
                )),
            )
        })
        .collect();
    for node in &mut document.nodes {
        if remapped.contains_key(&node.id)
            && !node
                .facts
                .iter()
                .any(|fact| fact.path.0 == "/retained_scope")
        {
            node.facts.push(super::normalize::unknown(
                "/retained_scope",
                ChangeKind::Declaration,
                UnknownReason::Unobserved,
            ));
        }
    }
    for node in &mut snapshot.nodes {
        if let Some(id) = remapped.get(&node.id) {
            node.id = id.clone();
        }
        node.facts.push(super::normalize::unknown(
            "/retained_scope",
            ChangeKind::Declaration,
            UnknownReason::Unobserved,
        ));
    }
    for edge in &mut snapshot.edges {
        if let Some(id) = remapped.get(&edge.dependency) {
            edge.dependency = id.clone();
        }
        if let Some(id) = remapped.get(&edge.consumer) {
            edge.consumer = id.clone();
        }
    }
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod tests;
