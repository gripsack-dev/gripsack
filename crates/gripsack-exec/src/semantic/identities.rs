//! Reuse the production package identity encoder; never derive recipes from review hashes.
use super::{
    model::*,
    normalize::{subject, unknown},
};
use gripsack_ir::workspace_model::{
    WorkspaceCatalog, WorkspaceOutput, WorkspaceProducer,
    identity::{DefinitionDigest, PackageDigest, provider_package_digest},
    lock::{WorkspaceLock, platform_key},
};
use std::collections::{BTreeMap, VecDeque};

pub(super) fn capture(
    workspace: &WorkspaceCatalog,
    lock: Option<&WorkspaceLock>,
    document: &mut SnapshotDocument,
) {
    let packages: BTreeMap<_, _> = workspace
        .outputs
        .iter()
        .filter_map(|output| {
            if let WorkspaceOutput::Package(package) = output {
                Some((package.name.as_str(), package))
            } else {
                None
            }
        })
        .collect();
    let mut pending: BTreeMap<_, usize> = packages
        .iter()
        .map(|(name, package)| (*name, package.runtime.len()))
        .collect();
    let mut users: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, package) in &packages {
        for runtime in &package.runtime {
            users.entry(runtime).or_default().push(name);
        }
    }
    let mut ready: VecDeque<_> = pending
        .iter()
        .filter_map(|(name, count)| (*count == 0).then_some(*name))
        .collect();
    let mut identities: BTreeMap<&str, PackageDigest> = BTreeMap::new();
    let redacted: std::collections::BTreeSet<_> = document
        .nodes
        .iter()
        .filter(|node| {
            node.facts.iter().any(|fact| {
                matches!(
                    fact.knowledge,
                    KnowledgeState::Unknown {
                        reason: UnknownReason::Redacted
                    }
                )
            })
        })
        .map(|node| node.id.clone())
        .collect();
    while let Some(name) = ready.pop_front() {
        let package = packages[name];
        if !redacted.contains(&subject("output", name))
            && let WorkspaceProducer::Provider { provider } = &package.producer
        {
            let pin = lock
                .and_then(|lock| lock.resolutions.get(&platform_key(&package.target)))
                .and_then(|resolution| {
                    resolution
                        .pins
                        .iter()
                        .find(|pin| pin.output == name && pin.source == provider.locked())
                });
            let runtime: Option<BTreeMap<_, _>> = package
                .runtime
                .iter()
                .map(|name| {
                    identities
                        .get(name.as_str())
                        .map(|identity| (name.clone(), *identity))
                })
                .collect();
            if let (Some(pin), Some(runtime)) = (pin, runtime)
                && let Ok(identity) = provider_package_digest(package, &pin.resolved, &runtime)
            {
                identities.insert(name, identity);
            }
        }
        for user in users.get(name).into_iter().flatten() {
            if let Some(count) = pending.get_mut(user) {
                *count -= 1;
                if *count == 0 {
                    ready.push_back(*user);
                }
            }
        }
    }
    let identities: BTreeMap<_, _> = identities
        .into_iter()
        .map(|(name, identity)| (subject("output", name), identity))
        .collect();
    for node in &mut document.nodes {
        if node.kind != SubjectKind::Package {
            continue;
        }
        let identity = identities.get(&node.id);
        node.facts.push(match identity {
            Some(identity) => Fact {
                path: FieldPath("/package_identity".into()),
                kind: ChangeKind::Materialization,
                knowledge: KnowledgeState::Known {
                    digest: DefinitionDigest::from_bytes(*identity.bytes()),
                },
                origin: EvidenceOrigin::Lock,
                observed_at: None,
            },
            None => unknown(
                "/package_identity",
                ChangeKind::Materialization,
                UnknownReason::MissingInput,
            ),
        });
    }
}
