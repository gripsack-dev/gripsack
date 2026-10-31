//! Evidence is projected from admitted in-memory records. No file reads, probes,
//! evaluation attempts or raw configuration values become comparison authority.
use super::{model::*, normalize::*};
use crate::Repository;
use gripsack_ir::{
    Ir,
    workspace_model::{InputOrigin, identity::DefinitionDigest},
};
use gripsack_store::source_bundle::{SourceInventory, SourceObject};
use serde_json::Value;

pub(crate) fn output_subject(name: &str) -> SubjectRef {
    subject("output", name)
}

pub(crate) fn is_redacted(node: &SnapshotNode) -> bool {
    node.facts.iter().any(|fact| {
        matches!(
            fact.knowledge,
            KnowledgeState::Unknown {
                reason: UnknownReason::Redacted
            }
        )
    })
}

pub(crate) fn record_identity(
    node: &mut SnapshotNode,
    path: &str,
    kind: ChangeKind,
    digest: DefinitionDigest,
    origin: EvidenceOrigin,
) {
    let prefix = format!("{path}/");
    node.facts
        .retain(|fact| fact.path.0 != path && !fact.path.0.starts_with(&prefix));
    node.facts.push(Fact {
        path: FieldPath(path.into()),
        kind,
        knowledge: KnowledgeState::Known { digest },
        origin,
        observed_at: None,
    });
}

pub(crate) fn record_value(
    node: &mut SnapshotNode,
    path: &str,
    kind: ChangeKind,
    value: Value,
    origin: EvidenceOrigin,
) {
    let prefix = format!("{path}/");
    node.facts
        .retain(|fact| fact.path.0 != path && !fact.path.0.starts_with(&prefix));
    node.facts.extend(facts(value, path, "", kind, origin));
}

pub(super) fn evaluated(
    ir: &Ir,
    repository: &Repository,
    document: &mut SnapshotDocument,
) -> Result<(), SnapshotError> {
    let Some(workspace) = document
        .nodes
        .iter_mut()
        .find(|node| node.kind == SubjectKind::Workspace)
    else {
        return Err(SnapshotError::Invalid("missing workspace evidence"));
    };
    if let Some(inputs) = repository.evaluation_inputs() {
        record_value(
            workspace,
            "/facts",
            ChangeKind::Eligibility,
            serde_json::to_value(inputs.facts())?,
            EvidenceOrigin::Observation,
        );
        record_value(
            workspace,
            "/probes",
            ChangeKind::Input,
            Value::Object(Default::default()),
            EvidenceOrigin::Observation,
        );
        for (name, result) in inputs.probes() {
            // Trusted probe results are booleans, never generic configuration
            // values. A name containing "token" does not make a boolean secret.
            workspace.facts.push(known(
                &format!("/probes/{}", escaped(name)),
                ChangeKind::Input,
                if *result { b"true" } else { b"false" },
                EvidenceOrigin::Observation,
            ));
        }
    }
    let Repository::Evaluated { sources, .. } = repository else {
        return Ok(());
    };
    // The complete source bundle is approval provenance only. Unrelated files,
    // config payloads and evaluation attempt IDs cannot alter a selected contract.
    record_value(
        workspace,
        "/source_bundle",
        ChangeKind::Provenance,
        Value::String(sources.digest().to_string()),
        EvidenceOrigin::Observation,
    );
    let definitions = crate::workspace::captured_definitions(repository)
        .map_err(|_| SnapshotError::Invalid("captured definitions unavailable"))?;
    record_value(
        workspace,
        "/definitions",
        ChangeKind::Input,
        serde_json::to_value(definitions)?,
        EvidenceOrigin::Observation,
    );
    document
        .nodes
        .sort_unstable_by(|left, right| left.id.cmp(&right.id));
    for input in &ir.workspace.inputs {
        let id = subject("input", &input.name);
        let Ok(index) = document.nodes.binary_search_by(|node| node.id.cmp(&id)) else {
            continue;
        };
        let node = &mut document.nodes[index];
        capture_input(node, &input.origin, sources.inventory(), repository)?;
    }
    for output in &ir.workspace.outputs {
        let id = subject("output", output.name());
        let Ok(index) = document.nodes.binary_search_by(|node| node.id.cmp(&id)) else {
            continue;
        };
        use gripsack_ir::workspace_model::{AcquisitionSource, WorkspaceOutput, WorkspaceProducer};
        let source = match output {
            WorkspaceOutput::Package(package) => match &package.producer {
                WorkspaceProducer::Provider { provider } => provider,
                _ => continue,
            },
            WorkspaceOutput::Recipe(recipe) => &recipe.source,
            _ => continue,
        };
        let AcquisitionSource::Fetch(fetch) = source else {
            continue;
        };
        let gripsack_ir::FetchSpec::File { path } = &fetch.fetch else {
            continue;
        };
        let Some(relative) = repository
            .source_relative(std::path::Path::new(path))
            .and_then(|path| path.to_str())
        else {
            continue;
        };
        if resolve(sources.inventory(), &format!("repo/{relative}")).is_none() {
            continue;
        }
        let node = &mut document.nodes[index];
        if node.facts.iter().any(|fact| {
            fact.path.0 == "/lock"
                && matches!(
                    fact.knowledge,
                    KnowledgeState::Unknown {
                        reason: UnknownReason::MissingLock
                    }
                )
        }) {
            record_value(
                node,
                "/lock",
                ChangeKind::Input,
                Value::Null,
                EvidenceOrigin::Observation,
            );
        }
    }
    Ok(())
}

/// Production pin admission understands source fields populated by resolution
/// (notably Pixi's captured manifest/lock identities).
pub(super) fn admitted_pins(
    ir: &Ir,
    pins: &crate::workspace::pins::WorkspacePins,
    document: &mut SnapshotDocument,
) -> Result<(), SnapshotError> {
    use gripsack_ir::workspace_model::{WorkspaceOutput, WorkspaceProducer};
    for output in &ir.workspace.outputs {
        let id = subject("output", output.name());
        let Some(node) = document.nodes.iter_mut().find(|node| node.id == id) else {
            continue;
        };
        if !node
            .facts
            .iter()
            .any(|fact| fact.path.0 == "/lock_declaration_binding")
        {
            continue;
        }
        let (source, target) = match output {
            WorkspaceOutput::Package(package) => match &package.producer {
                WorkspaceProducer::Provider { provider } => (provider, &package.target),
                _ => continue,
            },
            WorkspaceOutput::Recipe(recipe) => (&recipe.source, &recipe.target),
            _ => continue,
        };
        if pins
            .lookup(target, output.name(), &source.locked())
            .map_err(|_| SnapshotError::Invalid("selected lock declaration binding"))?
            .is_some()
        {
            record_value(
                node,
                "/lock_declaration_binding",
                ChangeKind::Eligibility,
                Value::Bool(true),
                EvidenceOrigin::Lock,
            );
        }
    }
    Ok(())
}

fn resolve<'a>(inventory: &'a SourceInventory, path: &str) -> Option<&'a SourceObject> {
    resolve_entry(inventory, path).map(|entry| &entry.object)
}

fn resolve_entry<'a>(
    inventory: &'a SourceInventory,
    path: &str,
) -> Option<&'a gripsack_store::source_bundle::SourceEntry> {
    let mut path = std::borrow::Cow::Borrowed(path);
    for _ in 0..MAX_DEPTH {
        if let Some(entry) = inventory.entry(&path) {
            match &entry.object {
                SourceObject::Alias { target } => path = std::borrow::Cow::Borrowed(target),
                _ => return Some(entry),
            }
        } else {
            // Inventory aliases name roots; their children exist only under the
            // resolved root, not at every logical alias spelling.
            let mut replacement = None;
            for (index, _) in path.match_indices('/') {
                if let Some(gripsack_store::source_bundle::SourceEntry {
                    object: SourceObject::Alias { target },
                    ..
                }) = inventory.entry(&path[..index])
                {
                    replacement = Some(format!("{target}{}", &path[index..]));
                    break;
                }
            }
            path = std::borrow::Cow::Owned(replacement?);
        }
    }
    None
}

fn capture_input(
    node: &mut SnapshotNode,
    origin: &InputOrigin,
    inventory: &SourceInventory,
    repository: &Repository,
) -> Result<(), SnapshotError> {
    match origin {
        InputOrigin::RepoFile { path } => {
            let Some(path) = repository
                .source_relative(std::path::Path::new(path))
                .and_then(|path| path.to_str())
            else {
                return Ok(());
            };
            if let Some(SourceObject::File { sha256, mode, .. }) =
                resolve(inventory, &format!("repo/{path}"))
            {
                record_identity(
                    node,
                    "/captured_bytes",
                    ChangeKind::Input,
                    DefinitionDigest::from_bytes(*sha256.as_bytes()),
                    EvidenceOrigin::Observation,
                );
                node.facts.push(known(
                    "/captured_bytes/executable",
                    ChangeKind::Input,
                    &[u8::from(mode.bits() & 0o111 != 0)],
                    EvidenceOrigin::Observation,
                ));
            }
        }
        InputOrigin::RepoDirectory {
            path,
            include,
            exclude,
        } => {
            let Some(path) = repository
                .source_relative(std::path::Path::new(path))
                .and_then(|path| path.to_str())
            else {
                return Ok(());
            };
            let root = if path == "." {
                "repo".to_owned()
            } else {
                format!("repo/{path}")
            };
            let Some(root) = resolve_entry(inventory, &root)
                .filter(|entry| matches!(entry.object, SourceObject::Directory))
            else {
                return Ok(());
            };
            let mut pending = vec![(root.path.clone(), String::new())];
            let mut captured = Vec::new();
            let mut visited = 0usize;
            while let Some((physical, logical)) = pending.pop() {
                visited += 1;
                if visited > MAX_FACTS {
                    return Err(SnapshotError::Bound("input objects"));
                }
                let covered = |prefix: &String| {
                    logical
                        .strip_prefix(prefix.as_str())
                        .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('/'))
                };
                if exclude.iter().any(covered) {
                    continue;
                }
                let selected = include.iter().any(covered);
                let ancestor = logical.is_empty()
                    || include.iter().any(|prefix| {
                        prefix
                            .strip_prefix(&logical)
                            .is_some_and(|suffix| suffix.starts_with('/'))
                    });
                if !selected && !ancestor {
                    continue;
                }
                let Some(entry) = inventory.entry(&physical) else {
                    return Ok(());
                };
                if captured.len() >= MAX_FACTS {
                    return Err(SnapshotError::Bound("facts"));
                }
                match &entry.object {
                    SourceObject::Alias { target } => pending.push((target.clone(), logical)),
                    SourceObject::Directory => {
                        if selected {
                            captured.push(known(
                                &format!("/captured_bytes/{}", escaped(&logical)),
                                ChangeKind::Input,
                                b"directory",
                                EvidenceOrigin::Observation,
                            ));
                        }
                        let prefix = format!("{physical}/");
                        let start = inventory
                            .entries()
                            .partition_point(|entry| entry.path.as_str() < prefix.as_str());
                        for child in inventory.entries()[start..]
                            .iter()
                            .take_while(|entry| entry.path.starts_with(&prefix))
                        {
                            let relative = &child.path[prefix.len()..];
                            if !relative.contains('/') {
                                let relative = if logical.is_empty() {
                                    relative.to_owned()
                                } else {
                                    format!("{logical}/{relative}")
                                };
                                pending.push((child.path.clone(), relative));
                            }
                        }
                    }
                    SourceObject::File { sha256, mode, .. } => {
                        if selected {
                            let path = format!("/captured_bytes/{}", escaped(&logical));
                            captured.push(Fact {
                                path: FieldPath(path.clone()),
                                kind: ChangeKind::Input,
                                knowledge: KnowledgeState::Known {
                                    digest: DefinitionDigest::from_bytes(*sha256.as_bytes()),
                                },
                                origin: EvidenceOrigin::Observation,
                                observed_at: None,
                            });
                            captured.push(known(
                                &format!("{path}/executable"),
                                ChangeKind::Input,
                                &[u8::from(mode.bits() & 0o111 != 0)],
                                EvidenceOrigin::Observation,
                            ));
                        }
                    }
                }
            }
            node.facts.retain(|fact| fact.path.0 != "/captured_bytes");
            node.facts.push(known(
                "/captured_bytes",
                ChangeKind::Input,
                b"captured-directory",
                EvidenceOrigin::Observation,
            ));
            node.facts.extend(captured);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
