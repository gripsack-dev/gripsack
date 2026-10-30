use super::model::*;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_REPORTED_CHANGES: usize = 10_000;
const MAX_CHANGE_LOCATIONS: usize = 4;

fn locations(node: &SnapshotNode) -> Vec<Provenance> {
    node.provenance
        .iter()
        .take(MAX_CHANGE_LOCATIONS)
        .cloned()
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Difference {
    Added,
    Removed,
    Changed,
    Unknown,
}
#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub subject: SubjectRef,
    pub field: FieldPath,
    pub kind: ChangeKind,
    pub difference: Difference,
    pub before: Option<KnowledgeState>,
    pub after: Option<KnowledgeState>,
    pub before_provenance: Vec<Provenance>,
    pub after_provenance: Vec<Provenance>,
    pub before_observed_at: Option<u64>,
    pub after_observed_at: Option<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactKind {
    NeedsRealization,
    RecomposeEnvironment,
    DeploymentPlan,
    RequiredValidation,
    ReevaluateEligibility,
    Invocation,
    Registration,
    PreparedJobRevision,
    ExecutionOrder,
    Uncertain,
    Observation,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Impact {
    pub subject: SubjectRef,
    pub kind: ImpactKind,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphSide {
    Before,
    After,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReasonEdge {
    pub dependency: SubjectRef,
    pub consumer: SubjectRef,
    pub edge: EdgeKind,
    pub graph: GraphSide,
    pub impact: ImpactKind,
}
#[derive(Debug, Clone, Serialize)]
pub struct ChangeSet {
    pub version: u32,
    pub changes: Vec<Change>,
    pub total_changes: usize,
    pub established_changes: usize,
    /// Complete set even when explanation witnesses are truncated.
    pub impacts: Vec<Impact>,
    pub reasons: Vec<ReasonEdge>,
    pub completeness: Completeness,
    pub explanations_truncated: bool,
}
impl ChangeSet {
    pub fn is_complete(&self) -> bool {
        self.completeness.complete
    }
    pub fn has_changes(&self) -> bool {
        self.established_changes != 0
    }
}
fn impact(kind: ChangeKind) -> Option<ImpactKind> {
    Some(match kind {
        ChangeKind::Provenance => return None,
        ChangeKind::Declaration | ChangeKind::Input | ChangeKind::SelectedArtifact => {
            ImpactKind::NeedsRealization
        }
        ChangeKind::Materialization | ChangeKind::Environment => ImpactKind::RecomposeEnvironment,
        ChangeKind::Deployment => ImpactKind::DeploymentPlan,
        ChangeKind::Validation => ImpactKind::RequiredValidation,
        ChangeKind::Eligibility => ImpactKind::ReevaluateEligibility,
        ChangeKind::Invocation => ImpactKind::Invocation,
        ChangeKind::Registration => ImpactKind::Registration,
        ChangeKind::JobRevision => ImpactKind::PreparedJobRevision,
        ChangeKind::Observation => ImpactKind::Observation,
        ChangeKind::Ordering => ImpactKind::ExecutionOrder,
    })
}
fn consumer_impact(node: &SnapshotNode) -> ImpactKind {
    match node.kind {
        SubjectKind::Deployment | SubjectKind::Profile => ImpactKind::DeploymentPlan,
        SubjectKind::Task | SubjectKind::Hook | SubjectKind::Service => ImpactKind::Invocation,
        SubjectKind::Schedule | SubjectKind::Job => ImpactKind::PreparedJobRevision,
        SubjectKind::Check => ImpactKind::RequiredValidation,
        _ => ImpactKind::RecomposeEnvironment,
    }
}
fn propagated(edge: EdgeKind, incoming: ImpactKind, node: &SnapshotNode) -> Option<ImpactKind> {
    if matches!(incoming, ImpactKind::Observation) {
        return None;
    }
    if edge == EdgeKind::Retention {
        return None;
    }
    if incoming == ImpactKind::Uncertain {
        return Some(ImpactKind::Uncertain);
    }
    if edge == EdgeKind::Ordering || incoming == ImpactKind::ExecutionOrder {
        return Some(ImpactKind::ExecutionOrder);
    }
    if incoming == ImpactKind::ReevaluateEligibility {
        return Some(incoming);
    }
    Some(match edge {
        EdgeKind::Production | EdgeKind::BuildInput => {
            if matches!(
                node.kind,
                SubjectKind::Task | SubjectKind::Hook | SubjectKind::Check | SubjectKind::Service
            ) {
                consumer_impact(node)
            } else {
                ImpactKind::NeedsRealization
            }
        }
        EdgeKind::Validation => ImpactKind::RequiredValidation,
        EdgeKind::TaskPrerequisite => consumer_impact(node),
        EdgeKind::Runtime => consumer_impact(node),
        EdgeKind::Ordering | EdgeKind::Retention => unreachable!(),
    })
}

fn missing_knowledge(
    node: Option<&SnapshotNode>,
    path: &FieldPath,
    snapshot: &SemanticSnapshot,
) -> Option<KnowledgeState> {
    if let Some(node) = node {
        let mut parent = path.0.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if let Ok(index) = node
                .facts
                .binary_search_by(|fact| fact.path.0.as_str().cmp(prefix))
                && !node.facts[index].knowledge.is_known()
            {
                return Some(node.facts[index].knowledge.clone());
            }
            parent = prefix;
        }
    }
    snapshot
        .0
        .completeness
        .unknown_reasons
        .contains(&UnknownReason::HistoricalDeclaration)
        .then_some(KnowledgeState::Unknown {
            reason: UnknownReason::HistoricalDeclaration,
        })
}
/// Pure comparison of admitted facts. Unknown-to-unknown is never equality evidence.
pub fn compare(before: &SemanticSnapshot, after: &SemanticSnapshot) -> ChangeSet {
    let old: BTreeMap<_, _> = before.0.nodes.iter().map(|node| (&node.id, node)).collect();
    let new: BTreeMap<_, _> = after.0.nodes.iter().map(|node| (&node.id, node)).collect();
    let ids: BTreeSet<_> = old.keys().chain(new.keys()).copied().collect();
    let mut changes = Vec::new();
    for id in ids {
        let left = old.get(id).copied();
        let right = new.get(id).copied();
        let old_facts: BTreeMap<_, _> = left
            .into_iter()
            .flat_map(|node| &node.facts)
            .map(|fact| (&fact.path, fact))
            .collect();
        let new_facts: BTreeMap<_, _> = right
            .into_iter()
            .flat_map(|node| &node.facts)
            .map(|fact| (&fact.path, fact))
            .collect();
        let paths: BTreeSet<_> = old_facts.keys().chain(new_facts.keys()).copied().collect();
        for path in paths {
            let a = old_facts.get(path).copied();
            let b = new_facts.get(path).copied();
            let left_knowledge = a
                .map(|fact| fact.knowledge.clone())
                .or_else(|| missing_knowledge(left, path, before));
            let right_knowledge = b
                .map(|fact| fact.knowledge.clone())
                .or_else(|| missing_knowledge(right, path, after));
            let uncertain = left_knowledge
                .as_ref()
                .is_some_and(|value| !value.is_known())
                || right_knowledge
                    .as_ref()
                    .is_some_and(|value| !value.is_known());
            let difference = if uncertain {
                Difference::Unknown
            } else if a.is_none() {
                Difference::Added
            } else if b.is_none() {
                Difference::Removed
            } else if a.map(|f| (&f.knowledge, f.kind)) == b.map(|f| (&f.knowledge, f.kind)) {
                continue;
            } else {
                Difference::Changed
            };
            changes.push(Change {
                subject: id.clone(),
                field: path.clone(),
                kind: b.or(a).expect("union field").kind,
                difference,
                before: left_knowledge,
                after: right_knowledge,
                before_provenance: left.map_or_else(Vec::new, locations),
                after_provenance: right.map_or_else(Vec::new, locations),
                before_observed_at: a.and_then(|f| f.observed_at),
                after_observed_at: b.and_then(|f| f.observed_at),
            });
        }
        if let (Some(before_node), Some(after_node)) = (left, right)
            && before_node.provenance != after_node.provenance
        {
            changes.push(Change {
                subject: id.clone(),
                field: FieldPath("/provenance".into()),
                kind: ChangeKind::Provenance,
                difference: Difference::Changed,
                before: None,
                after: None,
                before_provenance: locations(before_node),
                after_provenance: locations(after_node),
                before_observed_at: None,
                after_observed_at: None,
            });
        }
        if left.is_none() || right.is_none() || left.map(|n| n.kind) != right.map(|n| n.kind) {
            changes.push(Change {
                subject: id.clone(),
                field: FieldPath("/subject".into()),
                kind: ChangeKind::Declaration,
                difference: if left.is_none()
                    && before
                        .0
                        .completeness
                        .unknown_reasons
                        .contains(&UnknownReason::HistoricalDeclaration)
                    || right.is_none()
                        && after
                            .0
                            .completeness
                            .unknown_reasons
                            .contains(&UnknownReason::HistoricalDeclaration)
                {
                    Difference::Unknown
                } else if left.is_none() {
                    Difference::Added
                } else if right.is_none() {
                    Difference::Removed
                } else {
                    Difference::Changed
                },
                before: None,
                after: None,
                before_provenance: left.map_or_else(Vec::new, locations),
                after_provenance: right.map_or_else(Vec::new, locations),
                before_observed_at: None,
                after_observed_at: None,
            });
        }
    }
    let old_edges: BTreeSet<_> = before.0.edges.iter().collect();
    let new_edges: BTreeSet<_> = after.0.edges.iter().collect();
    for edge in old_edges.symmetric_difference(&new_edges) {
        let kind = match edge.kind {
            EdgeKind::Ordering => ChangeKind::Ordering,
            EdgeKind::Validation => ChangeKind::Validation,
            EdgeKind::TaskPrerequisite => ChangeKind::Invocation,
            EdgeKind::Runtime | EdgeKind::Retention => ChangeKind::Environment,
            EdgeKind::Production | EdgeKind::BuildInput => ChangeKind::Input,
        };
        changes.push(Change {
            subject: edge.consumer.clone(),
            field: FieldPath(format!("/edges/{:?}/{}", edge.kind, edge.dependency.0)),
            kind,
            difference: if old_edges.contains(edge) {
                Difference::Removed
            } else {
                Difference::Added
            },
            before: None,
            after: None,
            before_provenance: old
                .get(&edge.consumer)
                .map_or_else(Vec::new, |n| locations(n)),
            after_provenance: new
                .get(&edge.consumer)
                .map_or_else(Vec::new, |n| locations(n)),
            before_observed_at: None,
            after_observed_at: None,
        });
    }
    changes.sort_by(|a, b| (&a.subject, &a.field).cmp(&(&b.subject, &b.field)));
    let mut reached = BTreeSet::new();
    let mut pending = VecDeque::new();
    for change in &changes {
        let kind = if change.difference == Difference::Unknown {
            Some(ImpactKind::Uncertain)
        } else {
            impact(change.kind)
        };
        if let Some(mut kind) = kind {
            if kind == ImpactKind::NeedsRealization
                && let Some(node) = new
                    .get(&change.subject)
                    .or_else(|| old.get(&change.subject))
                && matches!(
                    node.kind,
                    SubjectKind::Profile
                        | SubjectKind::Deployment
                        | SubjectKind::Task
                        | SubjectKind::Hook
                        | SubjectKind::Service
                        | SubjectKind::Schedule
                        | SubjectKind::Job
                        | SubjectKind::Environment
                        | SubjectKind::Check
                )
            {
                kind = consumer_impact(node);
            }
            let item = Impact {
                subject: change.subject.clone(),
                kind,
            };
            if reached.insert(item.clone()) {
                pending.push_back(item);
            }
        }
    }
    let mut adjacency: BTreeMap<&SubjectRef, Vec<(&SnapshotEdge, GraphSide)>> = BTreeMap::new();
    for (snapshot, side) in [(before, GraphSide::Before), (after, GraphSide::After)] {
        for edge in &snapshot.0.edges {
            adjacency
                .entry(&edge.dependency)
                .or_default()
                .push((edge, side));
        }
    }
    let mut reasons = Vec::new();
    let mut truncated = before
        .0
        .nodes
        .iter()
        .chain(&after.0.nodes)
        .any(|node| node.provenance.len() > MAX_CHANGE_LOCATIONS);
    while let Some(item) = pending.pop_front() {
        for &(edge, graph) in adjacency.get(&item.subject).into_iter().flatten() {
            let nodes = if graph == GraphSide::Before {
                &old
            } else {
                &new
            };
            let Some(kind) = propagated(edge.kind, item.kind, nodes[&edge.consumer]) else {
                continue;
            };
            if reasons.len() < MAX_REASONS {
                reasons.push(ReasonEdge {
                    dependency: edge.dependency.clone(),
                    consumer: edge.consumer.clone(),
                    edge: edge.kind,
                    graph,
                    impact: kind,
                });
            } else {
                truncated = true;
            }
            let impact = Impact {
                subject: edge.consumer.clone(),
                kind,
            };
            if reached.insert(impact.clone()) {
                pending.push_back(impact);
            }
        }
    }
    let total_changes = changes.len();
    let established_changes = changes
        .iter()
        .filter(|change| change.difference != Difference::Unknown)
        .count();
    if changes.len() > MAX_REPORTED_CHANGES {
        changes.truncate(MAX_REPORTED_CHANGES);
        truncated = true;
    }
    let unknown_reasons = before
        .0
        .completeness
        .unknown_reasons
        .iter()
        .chain(&after.0.completeness.unknown_reasons)
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    ChangeSet {
        version: SNAPSHOT_VERSION,
        changes,
        total_changes,
        established_changes,
        impacts: reached.into_iter().collect(),
        reasons,
        completeness: Completeness {
            complete: before.0.completeness.complete && after.0.completeness.complete && !truncated,
            redacted: before.0.completeness.redacted || after.0.completeness.redacted,
            truncated: before.0.completeness.truncated
                || after.0.completeness.truncated
                || truncated,
            unknown_reasons,
        },
        explanations_truncated: truncated,
    }
}
